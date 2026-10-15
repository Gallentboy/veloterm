/*
 * telnet_main.c - VeloTerm 胶水：在 PuTTY 的 telnet 后端 + plink 的
 * console 会话主循环之上，提供一个 GNU/inetutils 风格参数面的 telnet 命令。
 *
 * 来源与许可：本文件派生自 PuTTY 源码树 windows/plink.c
 * （git.tartarus.org/simon/putty.git master @ 7507c54b，PuTTY 0.85 开发版，
 * MIT 许可，全文见 ../putty/LICENCE）。seat / handle / 主循环部分自
 * plink.c 逐字搬用（仅把 plink_ 前缀的 static 改名为 telnet_、去掉
 * telnet 命令语义下不可达的分支），命令行处理部分整体替换为本文件
 * 自己的标准 telnet 参数解析（任务要求：解析在 C 层原生完成，不走
 * PuTTY 的 cmdline.c 翻译层）。
 *
 * 与 windows/plink.c 的差异清单：
 *  - main 改名 putty_telnet_main（由 Rust 侧 FFI 调用）。
 *  - plink 的命令行循环（cmdline_process_param / -load / -ssh / 远程
 *    command 拼接等）整体替换为下方 telnet_parse_args：原生解析
 *      telnet [-4] [-6] [-8] [-E] [-K] [-L] [-a] [-d] [-b hostalias]
 *             [-e escapechar] [-l user] [-n tracefile] [host [port]]
 *  - 协议无条件强制 PROT_TELNET、端口缺省 23（telnet(1) 语义；plink 的
 *    PLINK_PROTOCOL 环境变量、-shareexists、-s、sanitise 开关、-V/-pgpfp
 *    等 plink 专属选项不保留）。
 *  - sk_init() 提前到参数解析之前：端口参数支持服务名（getservbyname，
 *    经 PuTTY 自己的 net_service_lookup()），该调用需要 WinSock 已初始化。
 *  - enable_dit() 调用被去掉：其真实实现在 ssh/sshrand.c（本 telnet-only
 *    构建不链接 SSH 代码；上游为无 crypto 目录的应用提供的
 *    stubs/no-dit.c 在 x64 上是空翻译单元，链接不出来）。DIT 仅与
 *    ARM 指针认证相关，对 x64 Windows 无意义。
 *  - cmdline_run_saved() 不再调用：plink 用它回放 cmdline.c 预存的
 *    -m/-L 等参数；本文件从不调用 cmdline_process_param，预存表恒空，
 *    调用只会是一个误导性的 no-op（cmdline.c 仍参与链接，因为
 *    console.c 引用 cmdline_seat_verbose / cmdline_get_passwd_input）。
 *  - exit 语义与 plink 一致：正常收尾走 cleanup_exit()（内含 sk_cleanup
 *    后 exit(code)），即退出码经由 C 层 exit 生效。
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include <stdarg.h>
#include <ctype.h>

#include "putty.h"
#include "ssh.h"
#include "storage.h"
#include "tree234.h"
#include "security-api.h"

/* 上游 plink.c 同名函数（console.c / cmdline.c 会调用），报错前缀改成
 * "telnet"，错误消息的样子与 BSD/GNU telnet 一致。 */
void cmdline_error(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    console_print_error_msg_fmt_v("telnet", fmt, ap);
    va_end(ap);
    exit(1);
}

static HANDLE inhandle, outhandle, errhandle;
static struct handle *stdin_handle, *stdout_handle, *stderr_handle;
static handle_sink stdout_hs, stderr_hs;
static StripCtrlChars *stdout_scc, *stderr_scc;
static BinarySink *stdout_bs, *stderr_bs;
static DWORD orig_console_mode;

static Backend *backend;
static LogContext *logctx;
static Conf *conf;

static void telnet_echoedit_update(Seat *seat, bool echo, bool edit)
{
    /* Update stdin read mode to reflect changes in line discipline. */
    DWORD mode;

    mode = ENABLE_PROCESSED_INPUT;
    if (echo)
        mode = mode | ENABLE_ECHO_INPUT;
    else
        mode = mode & ~ENABLE_ECHO_INPUT;
    if (edit)
        mode = mode | ENABLE_LINE_INPUT;
    else
        mode = mode & ~ENABLE_LINE_INPUT;
    SetConsoleMode(inhandle, mode);
}

static size_t telnet_output(
    Seat *seat, SeatOutputType type, const void *data, size_t len)
{
    bool is_stderr = type != SEAT_OUTPUT_STDOUT;
    BinarySink *bs = is_stderr ? stderr_bs : stdout_bs;
    put_data(bs, data, len);

    return handle_backlog(stdout_handle) + handle_backlog(stderr_handle);
}

static bool telnet_eof(Seat *seat)
{
    handle_write_eof(stdout_handle);
    return false;   /* do not respond to incoming EOF with outgoing */
}

static SeatPromptResult telnet_get_userpass_input(Seat *seat, prompts_t *p)
{
    /* telnet 的登录提示在带内以普通文本出现，几乎不会走到 seat 级
     * 提示；但保留 plink 的完整兜底（先试 -pw 类状态，再退 console）。 */
    static cmdline_get_passwd_input_state cmdline_state =
        CMDLINE_GET_PASSWD_INPUT_STATE_INIT;

    SeatPromptResult spr;
    spr = cmdline_get_passwd_input(p, &cmdline_state, false);
    if (spr.kind == SPRK_INCOMPLETE)
        spr = console_get_userpass_input(p);
    return spr;
}

static bool telnet_seat_interactive(Seat *seat)
{
    return (!*conf_get_str_ambi(conf, CONF_remote_cmd, NULL) &&
            !*conf_get_str_ambi(conf, CONF_remote_cmd2, NULL) &&
            !*conf_get_str(conf, CONF_ssh_nc_host));
}

static const SeatVtable telnet_seat_vt = {
    .output = telnet_output,
    .eof = telnet_eof,
    .sent = nullseat_sent,
    .banner = nullseat_banner_to_stderr,
    .get_userpass_input = telnet_get_userpass_input,
    .notify_session_started = nullseat_notify_session_started,
    .notify_remote_exit = nullseat_notify_remote_exit,
    .notify_remote_disconnect = nullseat_notify_remote_disconnect,
    .connection_fatal = console_connection_fatal,
    .nonfatal = console_nonfatal,
    .update_specials_menu = nullseat_update_specials_menu,
    .get_ttymode = nullseat_get_ttymode,
    .set_busy_status = nullseat_set_busy_status,
    .confirm_ssh_host_key = console_confirm_ssh_host_key,
    .confirm_weak_crypto_primitive = console_confirm_weak_crypto_primitive,
    .confirm_weak_cached_hostkey = console_confirm_weak_cached_hostkey,
    .prompt_descriptions = console_prompt_descriptions,
    .is_utf8 = nullseat_is_never_utf8,
    .echoedit_update = telnet_echoedit_update,
    .get_display = nullseat_get_display,
    .get_windowid = nullseat_get_windowid,
    .get_window_pixel_size = nullseat_get_window_pixel_size,
    .stripctrl_new = console_stripctrl_new,
    .set_trust_status = console_set_trust_status,
    .can_set_trust_status = console_can_set_trust_status,
    .has_mixed_input_stream = console_has_mixed_input_stream,
    .verbose = cmdline_seat_verbose,
    .interactive = telnet_seat_interactive,
    .get_cursor_position = nullseat_get_cursor_position,
};
static Seat telnet_seat[1] = {{ &telnet_seat_vt }};

static DWORD main_thread_id;

/*
 * telnet(1) 风格用法。参数面按 GNU inetutils telnet 的 synopsis 逐字
 * 给出；无 host 时打印到 stderr 并退出 1（本命令不做交互式 telnet>
 * 命令模式——那是 BSD telnet 前端的能力，PuTTY console 后端没有对应物）。
 */
static void usage(FILE *fp)
{
    fprintf(fp, "Usage: telnet [-4] [-6] [-8] [-E] [-K] [-L] [-a] [-d]"
                " [-b hostalias]\n");
    fprintf(fp, "              [-e escapechar] [-l user] [-n tracefile]"
                " [host [port]]\n");
}

size_t stdin_gotdata(struct handle *h, const void *data, size_t len, int err)
{
    if (err) {
        char buf[4096];
        FormatMessage(FORMAT_MESSAGE_FROM_SYSTEM, NULL, err, 0,
                      buf, lenof(buf), NULL);
        buf[lenof(buf)-1] = '\0';
        if (buf[strlen(buf)-1] == '\n')
            buf[strlen(buf)-1] = '\0';
        fprintf(stderr, "Unable to read from standard input: %s\n", buf);
        cleanup_exit(0);
    }

    noise_ultralight(NOISE_SOURCE_IOLEN, len);
    if (backend_connected(backend)) {
        if (len > 0) {
            backend_send(backend, data, len);
            return backend_sendbuffer(backend);
        } else {
            backend_special(backend, SS_EOF, 0);
            return 0;
        }
    } else
        return 0;
}

void stdouterr_sent(struct handle *h, size_t new_backlog, int err, bool close)
{
    if (close) {
        CloseHandle(outhandle);
        CloseHandle(errhandle);
        outhandle = errhandle = INVALID_HANDLE_VALUE;
    }

    if (err) {
        char buf[4096];
        FormatMessage(FORMAT_MESSAGE_FROM_SYSTEM, NULL, err, 0,
                      buf, lenof(buf), NULL);
        buf[lenof(buf)-1] = '\0';
        if (buf[strlen(buf)-1] == '\n')
            buf[strlen(buf)-1] = '\0';
        fprintf(stderr, "Unable to write to standard %s: %s\n",
                (h == stdout_handle ? "output" : "error"), buf);
        cleanup_exit(0);
    }

    if (backend_connected(backend)) {
        backend_unthrottle(backend, (handle_backlog(stdout_handle) +
                                     handle_backlog(stderr_handle)));
    }
}

const bool share_can_be_downstream = true;
const bool share_can_be_upstream = true;

const unsigned cmdline_tooltype =
    TOOLTYPE_HOST_ARG |
    TOOLTYPE_HOST_ARG_CAN_BE_SESSION |
    TOOLTYPE_HOST_ARG_PROTOCOL_PREFIX |
    TOOLTYPE_HOST_ARG_FROM_LAUNCHABLE_LOAD;

static bool sending;

static bool telnet_mainloop_pre(void *vctx, const HANDLE **extra_handles,
                               size_t *n_extra_handles)
{
    if (!sending && backend_sendok(backend)) {
        stdin_handle = handle_input_new(inhandle, stdin_gotdata, NULL,
                                        0);
        sending = true;
    }

    return true;
}

static bool telnet_mainloop_post(void *vctx, size_t extra_handle_index)
{
    if (sending)
        handle_unthrottle(stdin_handle, backend_sendbuffer(backend));

    if (!backend_connected(backend) &&
        handle_backlog(stdout_handle) + handle_backlog(stderr_handle) == 0)
        return false; /* we closed the connection */

    return true;
}

/* ── 标准 telnet 参数解析（GNU/inetutils 风格，C 层原生实现）───────────
 *
 * synopsis:
 *   telnet [-4] [-6] [-8] [-E] [-K] [-L] [-a] [-d] [-b hostalias]
 *          [-e escapechar] [-l user] [-n tracefile] [host [port]]
 *
 * GNU getopt 兼容细节：支持聚簇（-8a）、吸附值（-luser / -e^]）、
 * "--" 结束选项、以及把 lone "-" 当位置参数处理。带值选项值缺失按
 * getopt 惯例报 "option requires an argument" 退出 1。
 *
 * 各开关 → PuTTY Conf / 行为 的取舍（任务要求接受并保持兼容面，注释
 * 说明各自语义；PuTTY 无对应语义的不中断连接流程）：
 *  -4 / -6 → CONF_addressfamily = ADDRTYPE_IPV4 / ADDRTYPE_IPV6
 *            （同传时后者生效，与 getopt 顺序副作用一致）
 *  -8      → 8-bit 输入。PuTTY telnet 后端本身按 8 位工作、自动协商
 *            BINARY 选项（otherbackends/telnet.c 无 7 位路径），天然
 *            满足，接受即等效。
 *  -E      → 禁用转义字符。PuTTY console 前端（plink 系）从不解释
 *            任何转义字符，"禁用"即默认状态，接受即等效（静默）。
 *  -e char → 设置转义字符。PuTTY console 前端没有 telnet> 转义机制，
 *            无法继续等价 → stderr 提示后忽略（连接照常）。
 *  -K      → 禁止自动登录（不主动送出用户名/认证选项）。PuTTY 的
 *            自动登录面 = telnet.c 的 get_remote_username()：清空
 *            CONF_username 并关掉 CONF_username_from_env 后它返回
 *            NULL，服务器侧收不到 USER/AUTH 供给 → 语义等价。
 *  -L      → 8 位输出路径。PuTTY 输出恒为 8 位字节流，接受即等效。
 *  -a      → 尝试自动登录：CONF_username_from_env = true，未给 -l 时
 *            get_remote_username() 改取本地登录名（settings.c）。
 *  -d      → 调试/套接字跟踪开关（SO_DEBUG + 输入输出跟踪）。PuTTY
 *            无对应 Conf 开关，接受并忽略（不中断）。
 *  -b hostalias → 在 ~/.telnetrc 里按别名查主机。PuTTY 不读 telnetrc，
 *            接受并忽略该值（保持脚本兼容面）。
 *  -l user → CONF_username（PuTTY telnet 把它作为 USER 环境选项 /
 *            AUTH 供给发出，见 otherbackends/telnet.c）。
 *  -n tracefile → 跟踪记录文件。PuTTY 的会话日志由保存的配置驱动
 *            （CONF_logtype），与 telnet 跟踪文件语义不同 → stderr
 *            提示后忽略。
 *  host    → 位置参数 → CONF_host（经 prepare_session 再支持
 *            user@host 形式，plink 同款兜底）。
 *  port    → 位置参数 → CONF_port；十进制数字，或服务名（经 PuTTY
 *            net_service_lookup() → getservbyname；注意其 proto=NULL，
 *            tcp/udp 任一命中的服务名都接受）；缺省 23。
 */
struct telnet_args {
    const char *host;
    const char *portspec;      /* 原样保存，最后统一解析 */
};

static bool telnet_parse_args(int argc, char **argv, struct telnet_args *ta)
{
    bool end_of_options = false;
    int i;

    ta->host = NULL;
    ta->portspec = NULL;

    for (i = 1; i < argc; i++) {
        char *p = argv[i];

        if (end_of_options || p[0] != '-' || p[1] == '\0') {
            /* 位置参数：host，随后至多一个 port */
            if (ta->host == NULL) {
                ta->host = p;
            } else if (ta->portspec == NULL) {
                ta->portspec = p;
            } else {
                fprintf(stderr, "telnet: too many arguments\n");
                usage(stderr);
                return false;
            }
            continue;
        }

        if (!strcmp(p, "--")) {
            end_of_options = true;
            continue;
        }

        /* 以 '-' 开头：走 getopt 风格逐字符扫描（支持聚簇与吸附值） */
        {
            const char *c = p + 1;
            while (*c) {
                const char *value = NULL;
                bool needs_value = false;
                switch (*c) {
                  case '4':
                    conf_set_int(conf, CONF_addressfamily, ADDRTYPE_IPV4);
                    break;
                  case '6':
                    conf_set_int(conf, CONF_addressfamily, ADDRTYPE_IPV6);
                    break;
                  case '8':                  /* 8-bit 输入：PuTTY 恒 8 位 */
                  case 'L':                  /* 8 位输出：PuTTY 恒 8 位 */
                  case 'E':                  /* 禁转义：本就没有转义字符 */
                  case 'd':                  /* 调试开关：无对应 Conf，忽略 */
                    break;
                  case 'K':                  /* 禁自动登录：不送用户名 */
                    conf_set_str(conf, CONF_username, "");
                    conf_set_bool(conf, CONF_username_from_env, false);
                    break;
                  case 'a':                  /* 自动登录：可用本地用户名 */
                    conf_set_bool(conf, CONF_username_from_env, true);
                    break;
                  case 'e':                  /* 转义字符：无对应机制 */
                    needs_value = true;
                    break;
                  case 'l':                  /* 用户名 → CONF_username */
                    needs_value = true;
                    break;
                  case 'n':                  /* 跟踪文件：无对应语义 */
                    needs_value = true;
                    break;
                  case 'b':                  /* telnetrc 别名：无 telnetrc */
                    needs_value = true;
                    break;
                  default:
                    fprintf(stderr, "telnet: unknown option \"%s\"\n", p);
                    usage(stderr);
                    return false;
                }

                if (needs_value) {
                    c++;                      /* 跳过选项字母，看是否有吸附值 */
                    if (*c) {
                        value = c;
                    } else if (i + 1 < argc) {
                        value = argv[++i];
                    } else {
                        fprintf(stderr,
                                "telnet: option requires an argument -- '%c'\n",
                                c[-1]);
                        usage(stderr);
                        return false;
                    }
                    switch (c[-1]) {
                      case 'e':
                        fprintf(stderr, "telnet: warning: escape character "
                                "is not supported by this telnet; "
                                "\"%s\" ignored\n", value);
                        break;
                      case 'l':
                        conf_set_str(conf, CONF_username, value);
                        break;
                      case 'n':
                        fprintf(stderr, "telnet: warning: trace file "
                                "\"%s\" ignored (no telnet tracing in "
                                "this build)\n", value);
                        break;
                      case 'b':
                        /* .telnetrc 主机别名：PuTTY 不读 telnetrc，静默忽略 */
                        break;
                      default:
                        unreachable("unhandled value option");
                    }
                    break;                   /* 该 argv 已消费完毕 */
                }
                c++;
            }
        }
    }
    return true;
}

/*
 * 端口参数解析：十进制 1..65535，否则按服务名查（PuTTY 自己的
 * net_service_lookup：sk_init 之后可用）。失败返回 -1。
 */
static int telnet_parse_port(const char *spec)
{
    char *end = NULL;
    unsigned long port;

    if (!spec)
        return 23;

    port = strtoul(spec, &end, 10);
    if (end && *end == '\0' && end != spec && port >= 1 && port <= 65535)
        return (int)port;

    port = net_service_lookup(spec);
    if (port >= 1 && port <= 65535)
        return (int)port;

    return -1;
}

int putty_telnet_main(int argc, char **argv)
{
    int exitcode;
    int port;
    const struct BackendVtable *vt;
    struct telnet_args ta;

    dll_hijacking_protection();
    /* VeloTerm vendor 修改：不调用 enable_dit()（实现位于未链接的
     * ssh/sshrand.c；上游对无 crypto 目录的应用提供的 stubs/no-dit.c
     * 在非 ARM 目标是空翻译单元）。见文件头差异清单。 */

    /*
     * WinSock 尽早初始化：端口参数支持服务名（net_service_lookup），
     * 且 plink 原有的 "requires WinSock 2" 检查也依赖这里的指针绑定。
     */
    sk_init();
    if (p_WSAEventSelect == NULL) {
        fprintf(stderr, "telnet requires WinSock 2\n");
        return 1;
    }

    /*
     * 协议/端口缺省值按 telnet(1) 语义设定：PROT_TELNET / 23。
     */
    settings_set_default_protocol(PROT_TELNET);
    settings_set_default_port(23);

    conf = conf_new();
    do_defaults(NULL, conf);   /* PuTTY "Default Settings"（注册表） */

    /*
     * 标准 telnet 参数解析（见 telnet_parse_args 头注释的映射表）。
     */
    if (!telnet_parse_args(argc, argv, &ta))
        return 1;

    if (ta.host == NULL) {
        /* 不做交互式 telnet> 命令模式：无 host 即用法错误 */
        fprintf(stderr, "telnet: host name argument required\n");
        usage(stderr);
        return 1;
    }

    port = telnet_parse_port(ta.portspec);
    if (port < 0) {
        fprintf(stderr, "telnet: %s: bad port\n", ta.portspec);
        return 1;
    }

    /* 强制 telnet 协议并写入位置参数结果（覆盖保存设置里的任何
     * Protocol/Port/host/username 相关值中与之冲突的部分）。 */
    conf_set_int(conf, CONF_protocol, PROT_TELNET);
    conf_set_int(conf, CONF_port, port);
    conf_set_str(conf, CONF_host, ta.host);

    prepare_session(conf);

    /*
     * 选择后端。SSH=0/SERIAL=0/OTHERBACKENDS=1 的 be_list 下，
     * PROT_TELNET 必然命中 telnet_backend。
     */
    vt = backend_vt_from_proto(conf_get_int(conf, CONF_protocol));
    if (vt == NULL) {
        fprintf(stderr,
                "Internal fault: Unsupported protocol found\n");
        return 1;
    }

    if (vt->flags & BACKEND_NEEDS_TERMINAL) {
        fprintf(stderr,
                "telnet doesn't support %s, which needs terminal emulation\n",
                vt->displayname_lc);
        return 1;
    }

    logctx = log_init(console_cli_logpolicy, conf);

    if (restricted_acl()) {
        lp_eventlog(console_cli_logpolicy,
                    "Running with restricted process ACL");
    }

    inhandle = GetStdHandle(STD_INPUT_HANDLE);
    outhandle = GetStdHandle(STD_OUTPUT_HANDLE);
    errhandle = GetStdHandle(STD_ERROR_HANDLE);

    /*
     * Turn off ECHO and LINE input modes. We don't care if this
     * call fails, because we know we aren't necessarily running in
     * a console.
     */
    GetConsoleMode(inhandle, &orig_console_mode);
    SetConsoleMode(inhandle, ENABLE_PROCESSED_INPUT);

    /*
     * Pass the output handles to the handle-handling subsystem.
     * (The input one we leave until we're through the
     * authentication process.)
     */
    stdout_handle = handle_output_new(outhandle, stdouterr_sent, NULL, 0);
    stderr_handle = handle_output_new(errhandle, stdouterr_sent, NULL, 0);
    handle_sink_init(&stdout_hs, stdout_handle);
    handle_sink_init(&stderr_hs, stderr_handle);
    stdout_bs = BinarySink_UPCAST(&stdout_hs);
    stderr_bs = BinarySink_UPCAST(&stderr_hs);

    /*
     * Decide whether to sanitise control sequences out of standard
     * output and standard error.
     *
     * telnet 交互会话不设 CONF_nopty（保持 plink 的 AUTO 判定：仅在
     * 输出直连控制台且无远程终端请求时剥离控制字符——交互 telnet
     * 属于"我们请求了终端形态输出"的场景，与 plink 交互模式一致）。
     */
    if (is_console_handle(outhandle) &&
        conf_get_bool(conf, CONF_nopty)) {
        stdout_scc = stripctrl_new(stdout_bs, true, L'\0');
        stdout_bs = BinarySink_UPCAST(stdout_scc);
    }
    if (is_console_handle(errhandle) &&
        conf_get_bool(conf, CONF_nopty)) {
        stderr_scc = stripctrl_new(stderr_bs, true, L'\0');
        stderr_bs = BinarySink_UPCAST(stderr_scc);
    }

    /*
     * Start up the connection.
     */
    winselcli_setup();                 /* ensure event object exists */
    {
        char *error, *realhost;
        /* nodelay is only useful if stdin is a character device (console) */
        bool nodelay = conf_get_bool(conf, CONF_tcp_nodelay) &&
            (GetFileType(GetStdHandle(STD_INPUT_HANDLE)) == FILE_TYPE_CHAR);

        error = backend_init(vt, telnet_seat, &backend, logctx, conf,
                             conf_get_str(conf, CONF_host),
                             conf_get_int(conf, CONF_port),
                             &realhost, nodelay,
                             conf_get_bool(conf, CONF_tcp_keepalives));
        if (error) {
            fprintf(stderr, "Unable to open connection:\n%s", error);
            sfree(error);
            return 1;
        }
        ldisc_create(conf, NULL, backend, telnet_seat);
        sfree(realhost);
    }

    main_thread_id = GetCurrentThreadId();

    sending = false;

    cli_main_loop(telnet_mainloop_pre, telnet_mainloop_post, NULL);

    exitcode = backend_exitcode(backend);
    if (exitcode < 0) {
        fprintf(stderr, "Remote process exit code unavailable\n");
        exitcode = 1;                  /* this is an error condition */
    }
    cleanup_exit(exitcode);
    return 0;                          /* placate compiler warning */
}
