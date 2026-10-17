#include <assert.h>
#include <ctype.h>
#include <errno.h>
#include <libgen.h>
#ifdef HAVE_SETLOCALE
#include <locale.h>
#endif
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#ifdef WIN32
#include <windows.h>
#include <io.h>
#include <fcntl.h>
#include <processenv.h>
#include <shellapi.h>
#include <wchar.h>
#include <wtypes.h>
extern void jv_tsd_dtoa_ctx_init();
#endif

#ifdef HAVE_LIBONIG
#include <oniguruma.h>
#endif

#if !defined(HAVE_ISATTY) && defined(HAVE__ISATTY)
#undef isatty
#define isatty _isatty
#endif

#if defined(HAVE_ISATTY) || defined(HAVE__ISATTY)
#define USE_ISATTY
#endif

#include "jv.h"
#include "jq.h"
#include "util.h"
#include "src/version.h"
#include "src/config_opts.inc"

int jq_testsuite(jv lib_dirs, int verbose, int argc, char* argv[]);

/*
 * For a longer help message we could use a better option parsing
 * strategy, one that lets stack options.
 */
static void usage(int code, int keep_it_short) {
  FILE *f = stderr;

  if (code == 0)
    f = stdout;

  int ret = fprintf(f,
    "jq - commandline JSON processor [version %s]\n"
    "\nUsage:\tjq [options] <jq filter> [file...]\n"
    "\tjq [options] --args <jq filter> [strings...]\n"
    "\tjq [options] --jsonargs <jq filter> [JSON_TEXTS...]\n\n"
    "jq is a tool for processing JSON inputs, applying the given filter to\n"
    "its JSON text inputs and producing the filter's results as JSON on\n"
    "standard output.\n\n"
    "The simplest filter is ., which copies jq's input to its output\n"
    "unmodified except for formatting. For more advanced filters see\n"
    "the jq(1) manpage (\"man jq\") and/or https://jqlang.org/.\n\n"
    "Example:\n\n\t$ echo '{\"foo\": 0}' | jq .\n"
    "\t{\n\t  \"foo\": 0\n\t}\n\n", JQ_VERSION);
  if (keep_it_short) {
    fprintf(f, "For listing the command options, use jq --help.\n");
  } else {
    (void) fprintf(f,
      "Command options:\n"
      "  -n, --null-input          use `null` as the single input value;\n"
      "  -R, --raw-input           read each line as string instead of JSON;\n"
      "  -s, --slurp               read all inputs into an array and use it as\n"
      "                            the single input value;\n"
      "  -c, --compact-output      compact instead of pretty-printed output;\n"
      "  -r, --raw-output          output strings without escapes and quotes;\n"
      "      --raw-output0         implies -r and output NUL after each output;\n"
      "  -j, --join-output         implies -r and output without newline after\n"
      "                            each output;\n"
      "  -a, --ascii-output        output strings by only ASCII characters\n"
      "                            using escape sequences;\n"
      "  -S, --sort-keys           sort keys of each object on output;\n"
      "  -C, --color-output        colorize JSON output;\n"
      "  -M, --monochrome-output   disable colored output;\n"
      "      --tab                 use tabs for indentation;\n"
      "      --indent n            use n spaces for indentation (max 7 spaces);\n"
      "      --unbuffered          flush output stream after each output;\n"
      "      --stream              parse the input value in streaming fashion;\n"
      "      --stream-errors       implies --stream and report parse error as\n"
      "                            an array;\n"
      "      --seq                 parse input/output as application/json-seq;\n"
      "  -f, --from-file           load the filter from a file;\n"
      "  -L, --library-path dir    search modules from the directory;\n"
      "      --arg name value      set $name to the string value;\n"
      "      --argjson name value  set $name to the JSON value;\n"
      "      --slurpfile name file set $name to an array of JSON values read\n"
      "                            from the file;\n"
      "      --rawfile name file   set $name to string contents of file;\n"
      "      --args                consume remaining arguments as positional\n"
      "                            string values;\n"
      "      --jsonargs            consume remaining arguments as positional\n"
      "                            JSON values;\n"
      "  -e, --exit-status         set exit status code based on the output;\n"
#ifdef WIN32
      "  -b, --binary              open input/output streams in binary mode;\n"
#endif
      "  -V, --version             show the version;\n"
      "  --build-configuration     show jq's build configuration;\n"
      "  -h, --help                show the help;\n"
      "  --                        terminates argument processing;\n\n"
      "Named arguments are also available as $ARGS.named[], while\n"
      "positional arguments are available as $ARGS.positional[].\n");
  }
  exit((ret < 0 && code == 0) ? 2 : code);
}

static void die(void) {
  fprintf(stderr, "Use jq --help for help with command-line options,\n");
  fprintf(stderr, "or see the jq manpage, or online docs at https://jqlang.org\n");
  exit(2);
}

static int isoptish(const char* text) {
  return text[0] == '-' && (text[1] == '-' || isalpha((unsigned char)text[1]));
}

static int isoption(const char** text, char shortopt, const char* longopt, int is_short) {
  if (is_short) {
    if (shortopt && **text == shortopt) {
      (*text)++;
      if (!**text) *text = NULL;
      return 1;
    }
  } else {
    if (!strcmp(*text, longopt)) {
      *text = NULL;
      return 1;
    }
  }
  return 0;
}

enum {
  SLURP                 = 1,
  RAW_INPUT             = 2,
  PROVIDE_NULL          = 4,
  RAW_OUTPUT            = 8,
  RAW_OUTPUT0           = 16,
  ASCII_OUTPUT          = 32,
  COLOR_OUTPUT          = 64,
  NO_COLOR_OUTPUT       = 128,
  SORTED_OUTPUT         = 256,
  FROM_FILE             = 512,
  RAW_NO_LF             = 1024,
  UNBUFFERED_OUTPUT     = 2048,
  EXIT_STATUS           = 4096,
  SEQ                   = 16384,
  /* debugging only */
  DUMP_DISASM           = 32768,
};

enum {
    JQ_OK              =  0,
    JQ_OK_NULL_KIND    = -1, /* exit 0 if --exit-status is not set*/
    JQ_ERROR_SYSTEM    =  2,
    JQ_ERROR_COMPILE   =  3,
    JQ_OK_NO_OUTPUT    = -4, /* exit 0 if --exit-status is not set*/
    JQ_ERROR_UNKNOWN   =  5,
};
#define jq_exit_with_status(r)  exit(abs(r))
#define jq_exit(r)              exit( r > 0 ? r : 0 )

static int process(jq_state *jq, jv value, int flags, int dumpopts, int options) {
  int ret = JQ_OK_NO_OUTPUT; // No valid results && -e -> exit(4)
  jq_start(jq, value, flags);
  jv result;
  while (jv_is_valid(result = jq_next(jq))) {
    if ((options & RAW_OUTPUT) && jv_get_kind(result) == JV_KIND_STRING) {
      if (options & ASCII_OUTPUT) {
        jv_dumpf(jv_copy(result), stdout, JV_PRINT_ASCII);
      } else if ((options & RAW_OUTPUT0) && strlen(jv_string_value(result)) != (unsigned long)jv_string_length_bytes(jv_copy(result))) {
        jv_free(result);
        result = jv_invalid_with_msg(jv_string(
              "Cannot dump a string containing NUL with --raw-output0 option"));
        break;
      } else {
        priv_fwrite(jv_string_value(result), jv_string_length_bytes(jv_copy(result)),
            stdout, dumpopts & JV_PRINT_ISATTY);
      }
      ret = JQ_OK;
      jv_free(result);
    } else {
      if (jv_get_kind(result) == JV_KIND_FALSE || jv_get_kind(result) == JV_KIND_NULL)
        ret = JQ_OK_NULL_KIND;
      else
        ret = JQ_OK;
      if (options & SEQ)
        priv_fwrite("\036", 1, stdout, dumpopts & JV_PRINT_ISATTY);
      jv_dump(result, dumpopts);
    }
    if (!(options & RAW_NO_LF))
      priv_fwrite("\n", 1, stdout, dumpopts & JV_PRINT_ISATTY);
    if (options & RAW_OUTPUT0)
      priv_fwrite("\0", 1, stdout, dumpopts & JV_PRINT_ISATTY);
    if (options & UNBUFFERED_OUTPUT)
      fflush(stdout);
  }
  if (jq_halted(jq)) {
    // jq program invoked `halt` or `halt_error`
    jv exit_code = jq_get_exit_code(jq);
    if (!jv_is_valid(exit_code))
      ret = JQ_OK;
    else if (jv_get_kind(exit_code) == JV_KIND_NUMBER)
      ret = jv_number_value(exit_code);
    else
      ret = JQ_ERROR_UNKNOWN;
    jv_free(exit_code);
    jv error_message = jq_get_error_message(jq);
    if (jv_get_kind(error_message) == JV_KIND_STRING) {
      // No prefix should be added to the output of `halt_error`.
      priv_fwrite(jv_string_value(error_message), jv_string_length_bytes(jv_copy(error_message)),
          stderr, dumpopts & JV_PRINT_ISATTY);
    } else if (jv_get_kind(error_message) == JV_KIND_NULL) {
      // Halt with no output
    } else if (jv_is_valid(error_message)) {
      error_message = jv_dump_string(error_message, 0);
      fprintf(stderr, "%s\n", jv_string_value(error_message));
    } // else no message on stderr; use --debug-trace to see a message
    fflush(stderr);
    jv_free(error_message);
  } else if (jv_invalid_has_msg(jv_copy(result))) {
    // Uncaught jq exception
    jv msg = jv_invalid_get_msg(jv_copy(result));
    jv input_pos = jq_util_input_get_position(jq);
    if (jv_get_kind(msg) == JV_KIND_STRING) {
      fprintf(stderr, "jq: error (at %s): %s\n",
              jv_string_value(input_pos), jv_string_value(msg));
    } else {
      msg = jv_dump_string(msg, 0);
      fprintf(stderr, "jq: error (at %s) (not a string): %s\n",
              jv_string_value(input_pos), jv_string_value(msg));
    }
    ret = JQ_ERROR_UNKNOWN;
    jv_free(input_pos);
    jv_free(msg);
  }
  jv_free(result);
  return ret;
}

static void debug_cb(void *data, jv input) {
  int dumpopts = *(int *)data;
  jv_dumpf(JV_ARRAY(jv_string("DEBUG:"), input), stderr, dumpopts & ~(JV_PRINT_PRETTY));
  fprintf(stderr, "\n");
}

static void stderr_cb(void *data, jv input) {
  if (jv_get_kind(input) == JV_KIND_STRING) {
    int dumpopts = *(int *)data;
    priv_fwrite(jv_string_value(input), jv_string_length_bytes(jv_copy(input)),
        stderr, dumpopts & JV_PRINT_ISATTY);
  } else {
    input = jv_dump_string(input, 0);
    fprintf(stderr, "%s", jv_string_value(input));
  }
  jv_free(input);
}

#ifdef WIN32
int umain(int argc, char* argv[]);

/* veloterm 诊断插桩辅助（临时手段，定位后移除）：当前系统时间（100ns，
   FILETIME 纪元）。优先 GetSystemTimePreciseAsFileTime（Win8+，~100ns 精度；
   动态解析以免抬高 _WIN32_WINNT 要求），缺失时回退 GetSystemTimeAsFileTime
   （~15.6ms 粒度，仅作兜底）。 */
static ULONGLONG vt_now_100ns(void) {
  typedef VOID (WINAPI *vt_precise_fn)(LPFILETIME);
  static vt_precise_fn fn;
  static int resolved;
  FILETIME ft;
  if (!resolved) {
    resolved = 1;
    fn = (vt_precise_fn)(void *)GetProcAddress(GetModuleHandleA("kernel32.dll"),
                                               "GetSystemTimePreciseAsFileTime");
  }
  if (fn != NULL)
    fn(&ft);
  else
    GetSystemTimeAsFileTime(&ft);
  return ((ULONGLONG)ft.dwHighDateTime << 32) | (ULONGLONG)ft.dwLowDateTime;
}

/* veloterm 诊断插桩辅助：进程内存快照。PROCESS_MEMORY_COUNTERS_EX 在此本地
   定义（避开 psapi.h 对 _WIN32_WINNT 的门控），函数体经 kernel32 的
   K32GetProcessMemoryInfo 动态解析（不给链接器引入 psapi.lib）。 */
typedef struct vt_pmcex {
  DWORD  cb;
  DWORD  PageFaultCount;
  SIZE_T PeakWorkingSetSize;
  SIZE_T WorkingSetSize;
  SIZE_T QuotaPeakPagedPoolUsage;
  SIZE_T QuotaPagedPoolUsage;
  SIZE_T QuotaPeakNonPagedPoolUsage;
  SIZE_T QuotaNonPagedPoolUsage;
  SIZE_T PagefileUsage;
  SIZE_T PeakPagefileUsage;
  SIZE_T PrivateUsage;
} vt_pmcex;

static int vt_mem_snapshot(vt_pmcex *out) {
  typedef BOOL (WINAPI *vt_mem_fn)(HANDLE, vt_pmcex *, DWORD);
  static vt_mem_fn fn;
  static int resolved;
  memset(out, 0, sizeof(*out));
  out->cb = sizeof(*out);
  if (!resolved) {
    resolved = 1;
    fn = (vt_mem_fn)(void *)GetProcAddress(GetModuleHandleA("kernel32.dll"),
                                           "K32GetProcessMemoryInfo");
    if (fn == NULL) {
      HMODULE psapi = LoadLibraryA("psapi.dll");
      if (psapi != NULL)
        fn = (vt_mem_fn)(void *)GetProcAddress(psapi, "GetProcessMemoryInfo");
    }
  }
  if (fn == NULL)
    return 0;
  return fn(GetCurrentProcess(), out, sizeof(*out)) ? 1 : 0;
}

int wmain(int argc, wchar_t* wargv[]) {
  size_t arg_sz;
  char **argv = alloca(argc * sizeof(wchar_t*));
  for (int i = 0; i < argc; i++) {
    argv[i] = alloca((arg_sz = WideCharToMultiByte(CP_UTF8,
                                                   0,
                                                   wargv[i],
                                                   -1, 0, 0, 0, 0)));
    WideCharToMultiByte(CP_UTF8, 0, wargv[i], -1, argv[i], arg_sz, 0, 0);
  }
  return umain(argc, argv);
}

int umain(int argc, char* argv[]) {
#else /*}*/
int main(int argc, char* argv[]) {
#endif
  jq_state *jq = NULL;
  jq_util_input_state *input_state = NULL;
  int ret = JQ_OK_NO_OUTPUT;
  int compiled = 0;
  int parser_flags = 0;
  int nfiles = 0;
  int last_result = -1; /* -1 = no result, 0=null or false, 1=true */
  int badwrite;
  int options = 0;

#ifdef WIN32
  /* veloterm 诊断插桩：VELOTERM_JQ_TIMING=1 时在退出前向 stderr 输出分层数据
     ——in-main 分段 + 进程绝对时间线（pre_main=进程创建→umain 入口 /
     uptime=进程创建→探针点）+ 内存与 CPU 快照。当前用途是 test-jq.yml 的
     退出路径回归门禁（scripts/diagnose-jq-timing.ps1 驱动）：曾用它定位到
     「jq 输出数字后退出慢 ~450ms」的根因——进程以 0xC0000374
     （STATUS_HEAP_CORRUPTION）退出，WER 崩溃事后处理吃掉这段时间（详见
     jv.c 第 9 处补丁）。in-main 本身一直只有 ~2ms。
     VELOTERM_JQ_EXIT_MODE=quick|terminate 用于退出路径二分：
       quick     = _exit()（跳过 atexit 处理器与 stdio 冲刷）
       terminate = TerminateProcess（再跳过 CRT/加载器收尾，只剩内核回收地址空间）
     VELOTERM_JQ_EXIT_AFTER=compile 用于「编译后即退」对照：验证开销是否随
     编译期内存增长（而非执行/清理路径）出现。 */
  static LARGE_INTEGER vt_freq, vt_marks[5];
  static int vt_on;
  static int vt_exit_mode;            /* 0=normal / 1=quick(_exit) / 2=terminate */
  static ULONGLONG vt_creation_100ns; /* 进程创建时刻（FILETIME 100ns 纪元） */
  static ULONGLONG vt_entry_100ns;    /* umain 入口时刻（同源） */
#endif

#ifdef HAVE_SETLOCALE
  (void) setlocale(LC_ALL, "");
#endif

#ifdef HAVE_LIBONIG
  // use a lower regex parse depth limit than the default (4096) to protect
  // from stack-overflows
  // https://github.com/jqlang/jq/security/advisories/GHSA-f946-j5j2-4w5m
  onig_set_parse_depth_limit(1024);
#endif

#ifdef __OpenBSD__
  if (pledge("stdio rpath", NULL) == -1) {
    perror("pledge");
    exit(JQ_ERROR_SYSTEM);
  }
#endif

#ifdef WIN32
  jv_tsd_dtoa_ctx_init();
  fflush(stdout);
  fflush(stderr);
  vt_on = getenv("VELOTERM_JQ_TIMING") != NULL &&
          QueryPerformanceFrequency(&vt_freq) != 0;
  if (vt_on) {
    FILETIME vt_ct, vt_et, vt_kt, vt_ut;
    const char *vt_mode = getenv("VELOTERM_JQ_EXIT_MODE");
    QueryPerformanceCounter(&vt_marks[0]);
    vt_entry_100ns = vt_now_100ns();
    if (GetProcessTimes(GetCurrentProcess(), &vt_ct, &vt_et, &vt_kt, &vt_ut))
      vt_creation_100ns = ((ULONGLONG)vt_ct.dwHighDateTime << 32) | (ULONGLONG)vt_ct.dwLowDateTime;
    if (vt_mode != NULL) {
      if (strcmp(vt_mode, "quick") == 0)
        vt_exit_mode = 1;
      else if (strcmp(vt_mode, "terminate") == 0)
        vt_exit_mode = 2;
    }
  }
  /* veloterm 修补：删除上游对 stdout/stderr 的 `_O_TEXT | _O_U8TEXT` 设置。
     _O_U8TEXT 是 UCRT 的宽字符 UTF-8 模式，窄字符输出函数（fprintf/fwrite——
     jq 的全部错误输出与「重定向到管道/文件」的输出即此类）写该模式流属非法
     操作，release 版 CRT 经 invalid parameter handler 静默丢弃：表现为
     stderr 报错全被吞（坏输入时用户看到「完全无输出」而非 parse error）、
     `jq ... > file` 产出空文件。tty 下的正常结果输出走 jv_print.c put_buf 的
     WriteConsoleW，不经 stdio 模式，删本设置无影响；重定向路径回落为流默认
     的 _O_TEXT 窄字节直写，与上游官方 MinGW 构建行为一致（CRLF 翻译同源）。
     `-b/--binary` 的 _O_BINARY 分支保留不变。 */
#endif

  jv ARGS = jv_array(); /* positional arguments */
  jv program_arguments = jv_object(); /* named arguments */

  jq = jq_init();
  if (jq == NULL) {
    perror("jq_init");
    ret = JQ_ERROR_SYSTEM;
    goto out;
  }
#ifdef WIN32
  if (vt_on) QueryPerformanceCounter(&vt_marks[1]);
#endif

  int dumpopts = JV_PRINT_INDENT_FLAGS(2);
  const char* program = 0;

  input_state = jq_util_input_init(NULL, NULL); // XXX add err_cb

  int further_args_are_strings = 0;
  int further_args_are_json = 0;
  int args_done = 0;
  int jq_flags = 0;
  jv lib_search_paths = jv_null();
  for (int i=1; i<argc; i++) {
    if (args_done || !isoptish(argv[i])) {
      if (!program) {
        program = argv[i];
      } else if (further_args_are_strings) {
        ARGS = jv_array_append(ARGS, jv_string(argv[i]));
      } else if (further_args_are_json) {
        jv v =  jv_parse(argv[i]);
        if (!jv_is_valid(v)) {
          fprintf(stderr, "jq: invalid JSON text passed to --jsonargs\n");
          die();
        }
        ARGS = jv_array_append(ARGS, v);
      } else {
        jq_util_input_add_input(input_state, argv[i]);
        nfiles++;
      }
    } else if (!strcmp(argv[i], "--")) {
      args_done = 1;
    } else {
      const char* text = argv[i];
      int is_short;
      // First '-' already checked by isoptish
      if (text[1] == '-') {
        text += 2;
        is_short = 0;
      } else {
        text++;
        is_short = 1;
      }
      int raw; // Temporary for --rawfile

      // Parse a long option in one iteration or iterate over short options
      while (text != NULL) {
        if (isoption(&text, 's', "slurp", is_short)) {
          options |= SLURP;
        } else if (isoption(&text, 'r', "raw-output", is_short)) {
          options |= RAW_OUTPUT;
        } else if (isoption(&text, 0, "raw-output0", is_short)) {
          options |= RAW_OUTPUT | RAW_NO_LF | RAW_OUTPUT0;
        } else if (isoption(&text, 'j', "join-output", is_short)) {
          options |= RAW_OUTPUT | RAW_NO_LF;
        } else if (isoption(&text, 'c', "compact-output", is_short)) {
          dumpopts &= ~(JV_PRINT_TAB | JV_PRINT_INDENT_FLAGS(7));
        } else if (isoption(&text, 'C', "color-output", is_short)) {
          options |= COLOR_OUTPUT;
        } else if (isoption(&text, 'M', "monochrome-output", is_short)) {
          options |= NO_COLOR_OUTPUT;
        } else if (isoption(&text, 'a', "ascii-output", is_short)) {
          options |= ASCII_OUTPUT;
        } else if (isoption(&text, 0, "unbuffered", is_short)) {
          options |= UNBUFFERED_OUTPUT;
        } else if (isoption(&text, 'S', "sort-keys", is_short)) {
          options |= SORTED_OUTPUT;
        } else if (isoption(&text, 'R', "raw-input", is_short)) {
          options |= RAW_INPUT;
        } else if (isoption(&text, 'n', "null-input", is_short)) {
          options |= PROVIDE_NULL;
        } else if (isoption(&text, 'f', "from-file", is_short)) {
          options |= FROM_FILE;
        } else if (isoption(&text, 'L', "library-path", is_short)) {
          if (jv_get_kind(lib_search_paths) == JV_KIND_NULL)
            lib_search_paths = jv_array();
          if (text != NULL) { // -Lname
            lib_search_paths = jv_array_append(lib_search_paths, jq_realpath(jv_string(text)));
            text = NULL;
          } else if (i >= argc - 1) {
            fprintf(stderr, "-L takes a parameter: (e.g. -L /search/path or -L/search/path)\n");
            die();
          } else {
            lib_search_paths = jv_array_append(lib_search_paths, jq_realpath(jv_string(argv[i+1])));
            i++;
          }
        } else if (isoption(&text, 'b', "binary", is_short)) {
#ifdef WIN32
          fflush(stdout);
          fflush(stderr);
          _setmode(fileno(stdin),  _O_BINARY);
          _setmode(fileno(stdout), _O_BINARY);
          _setmode(fileno(stderr), _O_BINARY);
#endif
        } else if (isoption(&text, 0, "tab", is_short)) {
          dumpopts &= ~JV_PRINT_INDENT_FLAGS(7);
          dumpopts |= JV_PRINT_TAB | JV_PRINT_PRETTY;
        } else if (isoption(&text, 0, "indent", is_short)) {
          if (i >= argc - 1) {
            fprintf(stderr, "jq: --indent takes one parameter\n");
            die();
          }
          char* end = NULL;
          errno = 0;
          long indent = strtol(argv[i+1], &end, 10);
          if (errno || indent < -1 || indent > 7 ||
              isspace(*argv[i+1]) || end == argv[i+1] || *end) {
            fprintf(stderr, "jq: --indent takes a number between -1 and 7\n");
            die();
          }
          dumpopts &= ~(JV_PRINT_TAB | JV_PRINT_INDENT_FLAGS(7));
          dumpopts |= JV_PRINT_INDENT_FLAGS(indent);
          i++;
        } else if (isoption(&text, 0, "seq", is_short)) {
          options |= SEQ;
        } else if (isoption(&text, 0, "stream", is_short)) {
          parser_flags |= JV_PARSE_STREAMING;
        } else if (isoption(&text, 0, "stream-errors", is_short)) {
          parser_flags |= JV_PARSE_STREAMING | JV_PARSE_STREAM_ERRORS;
        } else if (isoption(&text, 'e', "exit-status", is_short)) {
          options |= EXIT_STATUS;
        } else if (isoption(&text, 0, "args", is_short)) {
          further_args_are_strings = 1;
          further_args_are_json = 0;
        } else if (isoption(&text, 0, "jsonargs", is_short)) {
          further_args_are_strings = 0;
          further_args_are_json = 1;
        } else if (isoption(&text, 0, "arg", is_short)) {
          if (i >= argc - 2) {
            fprintf(stderr, "jq: --arg takes two parameters (e.g. --arg varname value)\n");
            die();
          }
          if (!jv_object_has(jv_copy(program_arguments), jv_string(argv[i+1])))
            program_arguments = jv_object_set(program_arguments, jv_string(argv[i+1]), jv_string(argv[i+2]));
          i += 2; // skip the next two arguments
        } else if (isoption(&text, 0, "argjson", is_short)) {
          if (i >= argc - 2) {
            fprintf(stderr, "jq: --argjson takes two parameters (e.g. --argjson varname text)\n");
            die();
          }
          if (!jv_object_has(jv_copy(program_arguments), jv_string(argv[i+1]))) {
            jv v = jv_parse(argv[i+2]);
            if (!jv_is_valid(v)) {
              fprintf(stderr, "jq: invalid JSON text passed to --argjson\n");
              die();
            }
            program_arguments = jv_object_set(program_arguments, jv_string(argv[i+1]), v);
          }
          i += 2; // skip the next two arguments
        } else if ((raw = isoption(&text, 0, "rawfile", is_short)) ||
            isoption(&text, 0, "slurpfile", is_short)) {
          const char *which = raw ? "rawfile" : "slurpfile";
          if (i >= argc - 2) {
            fprintf(stderr, "jq: --%s takes two parameters (e.g. --%s varname filename)\n", which, which);
            die();
          }
          if (!jv_object_has(jv_copy(program_arguments), jv_string(argv[i+1]))) {
            jv data = jv_load_file(argv[i+2], raw);
            if (!jv_is_valid(data)) {
              data = jv_invalid_get_msg(data);
              fprintf(stderr, "jq: Bad JSON in --%s %s %s: %s\n", which,
                      argv[i+1], argv[i+2], jv_string_value(data));
              jv_free(data);
              ret = JQ_ERROR_SYSTEM;
              goto out;
            }
            program_arguments = jv_object_set(program_arguments, jv_string(argv[i+1]), data);
          }
          i += 2; // skip the next two arguments
        } else if (isoption(&text,  0,  "debug-dump-disasm", is_short)) {
          options |= DUMP_DISASM;
        } else if (isoption(&text,  0,  "debug-trace=all", is_short)) {
          jq_flags |= JQ_DEBUG_TRACE_ALL;
        } else if (isoption(&text,  0,  "debug-trace", is_short)) {
          jq_flags |= JQ_DEBUG_TRACE;
        } else if (isoption(&text, 'h', "help", is_short)) {
          usage(0, 0);
        } else if (isoption(&text, 'V', "version", is_short)) {
          printf("jq-%s\n", JQ_VERSION);
          ret = JQ_OK;
          goto out;
        } else if (isoption(&text, 0, "build-configuration", is_short)) {
          printf("%s\n", JQ_CONFIG);
          ret = JQ_OK;
          goto out;
        } else if (isoption(&text, 0, "run-tests", is_short)) {
          i++;
          // XXX Pass program_arguments, even a whole jq_state *, through;
          // could be useful for testing
          ret = jq_testsuite(lib_search_paths,
                             (options & DUMP_DISASM) || (jq_flags & JQ_DEBUG_TRACE),
                             argc - i, argv + i);
          goto out;
        } else {
          if (is_short) {
            fprintf(stderr, "jq: Unknown option -%c\n", text[0]);
          } else {
            fprintf(stderr, "jq: Unknown option --%s\n", text);
          }
          die();
        }
      }
    }
  }
#ifdef WIN32
  if (vt_on) QueryPerformanceCounter(&vt_marks[2]);
#endif

#ifdef USE_ISATTY
  if (isatty(STDOUT_FILENO)) {
#ifndef WIN32
    dumpopts |= JV_PRINT_ISATTY | JV_PRINT_COLOR;
#else
  /* Verify we actually have the console, as the NUL device is also regarded as
     tty.  Windows can handle color if ANSICON (or ConEmu) is installed, or
     Windows 10 supports the virtual terminal */
    DWORD mode;
    HANDLE con = GetStdHandle(STD_OUTPUT_HANDLE);
    if (GetConsoleMode(con, &mode)) {
      dumpopts |= JV_PRINT_ISATTY;
      if (getenv("ANSICON") != NULL ||
          SetConsoleMode(con, mode | 4/*ENABLE_VIRTUAL_TERMINAL_PROCESSING*/))
        dumpopts |= JV_PRINT_COLOR;
    }
#endif
    if (dumpopts & JV_PRINT_COLOR) {
      char *no_color = getenv("NO_COLOR");
      if (no_color != NULL && no_color[0] != '\0')
        dumpopts &= ~JV_PRINT_COLOR;
    }
  }
#endif
  if (options & SORTED_OUTPUT) dumpopts |= JV_PRINT_SORTED;
  if (options & ASCII_OUTPUT) dumpopts |= JV_PRINT_ASCII;
  if (options & COLOR_OUTPUT) dumpopts |= JV_PRINT_COLOR;
  if (options & NO_COLOR_OUTPUT) dumpopts &= ~JV_PRINT_COLOR;

  if (!jq_set_colors(getenv("JQ_COLORS")))
      fprintf(stderr, "Failed to set $JQ_COLORS\n");

  if (jv_get_kind(lib_search_paths) == JV_KIND_NULL) {
    // Default search path list
    lib_search_paths = JV_ARRAY(jv_string("~/.jq"),
                                jv_string("$ORIGIN/../lib/jq"),
                                jv_string("$ORIGIN/../lib"));
  }
  jq_set_attr(jq, jv_string("JQ_LIBRARY_PATH"), lib_search_paths);

  char *origin = strdup(argv[0]);
  if (origin == NULL) {
    fprintf(stderr, "jq: error: out of memory\n");
    exit(1);
  }
  jq_set_attr(jq, jv_string("JQ_ORIGIN"), jv_string(dirname(origin)));
  free(origin);

  if (strchr(JQ_VERSION, '-') == NULL)
    jq_set_attr(jq, jv_string("VERSION_DIR"), jv_string(JQ_VERSION));
  else
    jq_set_attr(jq, jv_string("VERSION_DIR"), jv_string_fmt("%.*s-master", (int)(strchr(JQ_VERSION, '-') - JQ_VERSION), JQ_VERSION));

#ifdef USE_ISATTY
  if (!program && !(options & FROM_FILE) && (!isatty(STDOUT_FILENO) || !isatty(STDIN_FILENO)))
    program = ".";
#endif

  if (!program) usage(2, 1);

  if (options & FROM_FILE) {
    char *program_origin = strdup(program);
    if (program_origin == NULL) {
      perror("malloc");
      exit(2);
    }

    jv data = jv_load_file(program, 1);
    if (!jv_is_valid(data)) {
      data = jv_invalid_get_msg(data);
      fprintf(stderr, "jq: %s\n", jv_string_value(data));
      free(program_origin);
      jv_free(data);
      ret = JQ_ERROR_SYSTEM;
      goto out;
    }
    int len = jv_string_length_bytes(jv_copy(data));
    if ((size_t)len != strlen(jv_string_value(data))) {
      fprintf(stderr, "jq: program file contains NUL bytes\n");
      free(program_origin);
      jv_free(data);
      ret = JQ_ERROR_SYSTEM;
      goto out;
    }
    jq_set_attr(jq, jv_string("PROGRAM_ORIGIN"), jq_realpath(jv_string(dirname(program_origin))));
    ARGS = JV_OBJECT(jv_string("positional"), ARGS,
                     jv_string("named"), jv_copy(program_arguments));
    program_arguments = jv_object_set(program_arguments, jv_string("ARGS"), jv_copy(ARGS));
    if (!jv_object_has(jv_copy(program_arguments), jv_string("JQ_BUILD_CONFIGURATION")))
      program_arguments = jv_object_set(program_arguments,
                                        jv_string("JQ_BUILD_CONFIGURATION"),
                                        jv_string(JQ_CONFIG)); /* named arguments */
    compiled = jq_compile_args(jq, jv_string_value(data), jv_copy(program_arguments));
    free(program_origin);
    jv_free(data);
  } else {
    jq_set_attr(jq, jv_string("PROGRAM_ORIGIN"), jq_realpath(jv_string("."))); // XXX is this good?
    ARGS = JV_OBJECT(jv_string("positional"), ARGS,
                     jv_string("named"), jv_copy(program_arguments));
    program_arguments = jv_object_set(program_arguments, jv_string("ARGS"), jv_copy(ARGS));
    if (!jv_object_has(jv_copy(program_arguments), jv_string("JQ_BUILD_CONFIGURATION")))
      program_arguments = jv_object_set(program_arguments,
                                        jv_string("JQ_BUILD_CONFIGURATION"),
                                        jv_string(JQ_CONFIG)); /* named arguments */
    compiled = jq_compile_args(jq, program, jv_copy(program_arguments));
  }
#ifdef WIN32
  if (vt_on) QueryPerformanceCounter(&vt_marks[3]);
#endif
  if (!compiled){
    ret = JQ_ERROR_COMPILE;
    goto out;
  }
#ifdef WIN32
  /* veloterm 诊断：编译后即退（对照实验，验证开销是否随编译期内存增长出现） */
  if (vt_on && getenv("VELOTERM_JQ_EXIT_AFTER") != NULL)
    goto out;
#endif

  if (options & DUMP_DISASM) {
    jq_dump_disassembly(jq, 0);
    printf("\n");
  }

  if ((options & SEQ))
    parser_flags |= JV_PARSE_SEQ;

  if ((options & RAW_INPUT))
    jq_util_input_set_parser(input_state, NULL, (options & SLURP) ? 1 : 0);
  else
    jq_util_input_set_parser(input_state, jv_parser_new(parser_flags), (options & SLURP) ? 1 : 0);

  // Let jq program read from inputs
  jq_set_input_cb(jq, jq_util_input_next_input_cb, input_state);

  // Let jq program call `debug` builtin and have that go somewhere
  jq_set_debug_cb(jq, debug_cb, &dumpopts);

  // Let jq program call `stderr` builtin and have that go somewhere
  jq_set_stderr_cb(jq, stderr_cb, &dumpopts);

  if (nfiles == 0)
    jq_util_input_add_input(input_state, "-");

  if (options & PROVIDE_NULL) {
    ret = process(jq, jv_null(), jq_flags, dumpopts, options);
  } else {
    jv value;
    while (jq_util_input_errors(input_state) == 0 &&
           (jv_is_valid((value = jq_util_input_next_input(input_state))) || jv_invalid_has_msg(jv_copy(value)))) {
      if (jv_is_valid(value)) {
        ret = process(jq, value, jq_flags, dumpopts, options);
        if (ret <= 0 && ret != JQ_OK_NO_OUTPUT)
          last_result = (ret != JQ_OK_NULL_KIND);
        if (jq_halted(jq))
          break;
        continue;
      }

      // Parse error
      jv msg = jv_invalid_get_msg(value);
      if (!(options & SEQ)) {
        ret = JQ_ERROR_UNKNOWN;
        fprintf(stderr, "jq: parse error: %s\n", jv_string_value(msg));
        jv_free(msg);
        break;
      }
      // --seq -> errors are not fatal
      fprintf(stderr, "jq: ignoring parse error: %s\n", jv_string_value(msg));
      jv_free(msg);
    }
  }

  if (jq_util_input_errors(input_state) != 0)
    ret = JQ_ERROR_SYSTEM;

out:
#ifdef WIN32
  if (vt_on) QueryPerformanceCounter(&vt_marks[4]);
#endif
  badwrite = ferror(stdout);
  if (fclose(stdout)!=0 || badwrite) {
    fprintf(stderr,"jq: error: writing output failed: %s\n", strerror(errno));
    ret = JQ_ERROR_SYSTEM;
  }

  jv_free(ARGS);
  jv_free(program_arguments);
  jq_util_input_free(&input_state);
  jq_teardown(&jq);

#ifdef WIN32
  if (vt_on) {
    LARGE_INTEGER vt_end;
    double vt_ms[6];
    int vt_i;
    ULONGLONG vt_now, vt_pre_main = 0, vt_uptime = 0;
    double vt_ws = 0.0, vt_peak_ws = 0.0, vt_private = 0.0;
    double vt_cpu_user = 0.0, vt_cpu_kernel = 0.0;
    DWORD vt_handles = 0;
    vt_pmcex vt_mem;
    FILETIME vt_ct, vt_et, vt_kt, vt_ut;
    QueryPerformanceCounter(&vt_end);
    vt_ms[0] = (vt_marks[1].QuadPart - vt_marks[0].QuadPart) * 1000.0 / vt_freq.QuadPart;
    vt_ms[1] = (vt_marks[2].QuadPart - vt_marks[1].QuadPart) * 1000.0 / vt_freq.QuadPart;
    vt_ms[2] = (vt_marks[3].QuadPart - vt_marks[2].QuadPart) * 1000.0 / vt_freq.QuadPart;
    vt_ms[3] = (vt_marks[4].QuadPart - vt_marks[3].QuadPart) * 1000.0 / vt_freq.QuadPart;
    vt_ms[4] = (vt_end.QuadPart - vt_marks[4].QuadPart) * 1000.0 / vt_freq.QuadPart;
    vt_ms[5] = (vt_end.QuadPart - vt_marks[0].QuadPart) * 1000.0 / vt_freq.QuadPart;
    /* 绝对时间线：把「进程创建→入口」与「入口→探针点」也量化，
       两者之差即退出路径 + 进程消亡的开销（由外层脚本用进程寿命对照）。 */
    vt_now = vt_now_100ns();
    if (vt_creation_100ns != 0) {
      if (vt_entry_100ns >= vt_creation_100ns)
        vt_pre_main = (vt_entry_100ns - vt_creation_100ns) / 10000ULL;
      if (vt_now >= vt_creation_100ns)
        vt_uptime = (vt_now - vt_creation_100ns) / 10000ULL;
    }
    memset(&vt_mem, 0, sizeof(vt_mem));
    if (vt_mem_snapshot(&vt_mem)) {
      vt_ws = vt_mem.WorkingSetSize / 1048576.0;
      vt_peak_ws = vt_mem.PeakWorkingSetSize / 1048576.0;
      vt_private = vt_mem.PrivateUsage / 1048576.0;
    }
    if (GetProcessTimes(GetCurrentProcess(), &vt_ct, &vt_et, &vt_kt, &vt_ut)) {
      vt_cpu_user = (((ULONGLONG)vt_ut.dwHighDateTime << 32) | vt_ut.dwLowDateTime) / 10000.0;
      vt_cpu_kernel = (((ULONGLONG)vt_kt.dwHighDateTime << 32) | vt_kt.dwLowDateTime) / 10000.0;
    }
    GetProcessHandleCount(GetCurrentProcess(), &vt_handles);
    fprintf(stderr, "[veloterm-jq-timing] init=%.1fms argparse=%.1fms compile=%.1fms "
                    "exec+output=%.1fms cleanup=%.1fms total=%.1fms | "
                    "pre_main=%llums uptime=%llums ws=%.1fMB peak_ws=%.1fMB private=%.1fMB "
                    "cpu=%.1fms(u %.1f/k %.1f) handles=%lu exit_mode=%d\n",
            vt_ms[0], vt_ms[1], vt_ms[2], vt_ms[3], vt_ms[4], vt_ms[5],
            (unsigned long long)vt_pre_main, (unsigned long long)vt_uptime,
            vt_ws, vt_peak_ws, vt_private,
            vt_cpu_user + vt_cpu_kernel, vt_cpu_user, vt_cpu_kernel,
            (unsigned long)vt_handles, vt_exit_mode);
    for (vt_i = 0; vt_i < 5; vt_i++) {
      if (vt_marks[vt_i].QuadPart == 0) {
        fprintf(stderr, "[veloterm-jq-timing] NOTE: 提前退出路径，部分分段未覆盖\n");
        break;
      }
    }
  }
#endif

#ifdef WIN32
  /* veloterm 诊断：退出路径二分（VELOTERM_JQ_EXIT_MODE=quick|terminate）。
     放在正常退出分发之前，仅探针启用时生效。 */
  if (vt_on && vt_exit_mode != 0) {
    int vt_code = ret > 0 ? ret : 0;
    if (vt_exit_mode == 2)
      TerminateProcess(GetCurrentProcess(), (UINT)vt_code);
    else
      _exit(vt_code);
  }
#endif

  if (options & EXIT_STATUS) {
    if (ret != JQ_OK_NO_OUTPUT)
      jq_exit_with_status(ret);
    else
      switch (last_result) {
        case -1: jq_exit_with_status(JQ_OK_NO_OUTPUT);
        case  0: jq_exit_with_status(JQ_OK_NULL_KIND);
        default: jq_exit_with_status(JQ_OK);
      }
  } else
    jq_exit(ret);
}
