/*
 * gif2mp4 — GIF → MP4(H.264) 转码实现。
 *
 * 管线（libavfilter 路线，与 CLI `ffmpeg -i in.gif -c:v libx264 ...` 行为对齐）：
 *
 *   gif demux → gif decode → filter graph → libx264(crf/preset) → mp4 mux(+faststart)
 *   graph = [fps=N,] scale=trunc(iw/2)*2:trunc(ih/2)*2, format=yuv420p
 *
 * - fps 给定 → CFR（重写 pts，规避 GIF 0 延迟帧/10ms 量化的时间戳坑）；
 *   缺省 → VFR 直通（sink pts 换算到编码器时基）。
 * - scale 表达式把宽高规整到偶数（yuv420p 的 4:2:0 采样要求）。
 * - 进度：两遍法——第一遍只 demux 数包得精确总帧数（毫秒级，远廉于
 *   解码）；正式遍按「每 16 帧 + 首帧 + 末帧」节流上报；回调返回非 0
 *   即取消（帧级粒度）。
 * - 原子性：全程写 "<output>.tmp"，trailer 落定后原子替换（Windows 用
 *   MoveFileExW REPLACE_EXISTING——MSVC 的 rename() 不覆盖既有目标）。
 * - 跨平台：文件路径全程 UTF-8；Windows 侧经 MultiByteToWideChar 转宽
 *   （ffmpeg 自身 file 协议在 Windows 同样按 UTF-8 处理，行为一致）。
 * - 防崩溃：所有回调发射先判空；av_log 未注册回调时转发原生默认回调；
 *   绝不 exit()/abort()。
 *
 * 本文件是 ffmpeg 树内库（ffbuild 构建，见同目录 Makefile 与 configure/
 * Makefile 补丁），头文件包含用树内相对路径。
 */

#include <inttypes.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "config.h"

#include "libavcodec/avcodec.h"
#include "libavfilter/avfilter.h"
#include "libavfilter/buffersink.h"
#include "libavfilter/buffersrc.h"
#include "libavformat/avformat.h"
#include "libavutil/avstring.h"
#include "libavutil/dict.h"
#include "libavutil/error.h"
#include "libavutil/log.h"
#include "libavutil/mem.h"
#include "libavutil/opt.h"
#include "libavutil/rational.h"

#include "gif2mp4.h"

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif

/* ── 进度节流参数（见文件头；常量在此便于调整） ────────────────── */

#define PROGRESS_EVERY_FRAMES 16

/* ── preset 白名单（libx264 十档） ──────────────────────────────── */

static const char *const k_preset_whitelist[] = {
    "ultrafast", "superfast", "veryfast", "faster", "fast",
    "medium",    "slow",      "slower",   "veryslow", "placebo",
    NULL,
};

/* ── TLS 错误串（MSVC / GCC 双宏；每线程独立缓冲） ──────────────── */

#if defined(_WIN32)
#  define GIF2MP4_TLS __declspec(thread)
#elif defined(__GNUC__)
#  define GIF2MP4_TLS __thread
#else
#  define GIF2MP4_TLS
#endif

static GIF2MP4_TLS char g_error_buf[512] = { 0 };

/* ── 日志管道：av_log 重定向 + NULL 守卫（防崩溃硬性规则） ──────── */

/* volatile 指针存取：aligned 指针的读写本身原子（x86/arm64）；
 * 更换回调与转码并发属不支持操作（见头文件），最坏丢日志、不崩溃 */
static gif2mp4_log_cb volatile g_log_cb;
static void *volatile g_log_ud;
static volatile int g_log_min_level = GIF2MP4_LOG_INFO;
static volatile int g_log_installed;

/* 库自身事件的广播入口（无回调时静默——错误另有 last_error 出口）。
 * 短路序：先判回调与级别过滤，命中才做格式化——无回调/低级别时零开销 */
static void lib_log(gif2mp4_log_level level, const char *fmt, ...)
{
    char buf[512];
    va_list ap;
    gif2mp4_log_cb cb = g_log_cb;
    if (!cb || (int)level > g_log_min_level)
        return;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    cb((void *)g_log_ud, level, buf);
}

/* set_error：记录 TLS 错误串并经日志管道广播（错误码原样返回，便于
 * `return set_error(...)` 一行式） */
static int set_error(int code, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(g_error_buf, sizeof(g_error_buf), fmt, ap);
    va_end(ap);
    lib_log(GIF2MP4_LOG_ERROR, "%s", g_error_buf);
    return code;
}

const char *gif2mp4_last_error(void)
{
    return g_error_buf;
}

int gif2mp4_abi_version(void)
{
    return GIF2MP4_ABI_VERSION;
}

/* AV_LOG_* → GIF2MP4_LOG_*（宿主不接触 av 级别数字，ABI 稳定） */
static gif2mp4_log_level map_av_level(int level)
{
    if (level <= AV_LOG_ERROR)
        return GIF2MP4_LOG_ERROR;
    if (level <= AV_LOG_WARNING)
        return GIF2MP4_LOG_WARN;
    if (level <= AV_LOG_INFO)
        return GIF2MP4_LOG_INFO;
    if (level <= AV_LOG_VERBOSE)
        return GIF2MP4_LOG_VERBOSE;
    return GIF2MP4_LOG_DEBUG;
}

/* av_log 回调适配：格式化单行、去尾换行、判空转发。
 * 可能从 lib 内部线程并发进入——只读全局快照，不持锁。
 * 短路序：判空 → av 级 → 宿主级别过滤，都过了才格式化。 */
static void av_log_adapter(void *avcl, int level, const char *fmt, va_list vl)
{
    char line[1024];
    int prefix = 1;
    gif2mp4_log_cb cb = g_log_cb;
    gif2mp4_log_level lv;
    size_t len;

    if (!cb) {
        /* 未注册：原样交给 ffmpeg 默认回调（写 stderr），语义等价于
         * 从未重定向——不吞日志、更不会因空指针崩溃 */
        av_log_default_callback(avcl, level, fmt, vl);
        return;
    }
    if (level > av_log_get_level())
        return;
    lv = map_av_level(level);
    if ((int)lv > g_log_min_level)
        return;
    if (av_log_format_line2(avcl, level, fmt, vl, line, sizeof(line), &prefix) < 0)
        return;
    len = strlen(line);
    while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r'))
        line[--len] = '\0';
    if (len == 0)
        return;
    cb((void *)g_log_ud, lv, line);
}

static void install_log_adapter(void)
{
    if (!g_log_installed) {
        g_log_installed = 1;
        av_log_set_callback(av_log_adapter);
    }
}

void gif2mp4_set_log_callback(gif2mp4_log_cb cb, void *userdata,
                              gif2mp4_log_level min_level)
{
    g_log_cb       = cb;
    g_log_ud       = userdata;
    g_log_min_level = (int)min_level;
    install_log_adapter();
    lib_log(GIF2MP4_LOG_INFO, "gif2mp4: log callback %s (min_level=%d)",
            cb ? "installed" : "removed (fall back to av_log default)",
            (int)min_level);
}

/* ── 进程级单飞（跨平台 CAS，不阻塞排队） ──────────────────────── */

#if defined(_WIN32)
static volatile LONG g_busy;
static int busy_try_acquire(void)
{
    return InterlockedCompareExchange(&g_busy, 1, 0) == 0;
}
static void busy_release(void)
{
    InterlockedExchange(&g_busy, 0);
}
#else
static volatile int g_busy;
static int busy_try_acquire(void)
{
    return __sync_bool_compare_and_swap(&g_busy, 0, 1);
}
static void busy_release(void)
{
    __sync_lock_release(&g_busy);
}
#endif

/* ── 跨平台文件操作（UTF-8 路径；Windows 转宽字符） ────────────── */

#if defined(_WIN32)
static wchar_t *utf8_to_wide(const char *utf8)
{
    int wlen = MultiByteToWideChar(CP_UTF8, 0, utf8, -1, NULL, 0);
    wchar_t *wide;
    if (wlen <= 0)
        return NULL;
    wide = (wchar_t *)av_malloc(sizeof(wchar_t) * wlen);
    if (!wide)
        return NULL;
    if (MultiByteToWideChar(CP_UTF8, 0, utf8, -1, wide, wlen) != wlen) {
        av_free(wide);
        return NULL;
    }
    return wide;
}
#endif

/* 原子替换：目标存在则覆盖。POSIX rename() 语义即覆盖；MSVC 的
 * rename() 遇已存在目标失败，Windows 用 MoveFileExW。 */
static int platform_replace(const char *from, const char *to)
{
#if defined(_WIN32)
    wchar_t *wfrom = utf8_to_wide(from);
    wchar_t *wto = utf8_to_wide(to);
    int ok;
    if (!wfrom || !wto) {
        av_free(wfrom);
        av_free(wto);
        return -1;
    }
    ok = MoveFileExW(wfrom, wto, MOVEFILE_REPLACE_EXISTING) ? 0 : -1;
    av_free(wfrom);
    av_free(wto);
    return ok;
#else
    return rename(from, to);
#endif
}

static int platform_remove(const char *path)
{
#if defined(_WIN32)
    wchar_t *wide = utf8_to_wide(path);
    int ok;
    if (!wide)
        return remove(path); /* 宽字符转换失败的兜底（ANSI 语义） */
    ok = _wremove(wide);
    av_free(wide);
    return ok;
#else
    return remove(path);
#endif
}

/* ── 第一遍：只 demux 数包（精确总帧数；毫秒级） ────────────────── */

/* 返回 >=0 帧数；<0 错误码（已 set_error）。duration_out 秒，未知 -1。 */
static int64_t count_gif_frames(const char *input_path, double *duration_out)
{
    AVFormatContext *ic = NULL;
    AVPacket *pkt;
    int ret, vs;
    int64_t frames = 0;

    *duration_out = -1.0;
    ret = avformat_open_input(&ic, input_path, NULL, NULL);
    if (ret < 0) {
        char err[AV_ERROR_MAX_STRING_SIZE] = { 0 };
        av_strerror(ret, err, sizeof(err));
        set_error(GIF2MP4_E_INPUT, "open input failed: %s (%s)", err, input_path);
        return GIF2MP4_E_INPUT;
    }
    if (avformat_find_stream_info(ic, NULL) < 0) {
        avformat_close_input(&ic);
        set_error(GIF2MP4_E_INPUT, "probe input failed: %s", input_path);
        return GIF2MP4_E_INPUT;
    }
    vs = av_find_best_stream(ic, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    if (vs < 0) {
        avformat_close_input(&ic);
        set_error(GIF2MP4_E_INPUT, "no video stream: %s", input_path);
        return GIF2MP4_E_INPUT;
    }
    if (ic->duration > 0 && ic->duration != AV_NOPTS_VALUE)
        *duration_out = ic->duration / (double)AV_TIME_BASE;

    pkt = av_packet_alloc();
    if (!pkt) {
        avformat_close_input(&ic);
        set_error(GIF2MP4_E_INTERNAL, "out of memory");
        return GIF2MP4_E_INTERNAL;
    }
    while ((ret = av_read_frame(ic, pkt)) >= 0) {
        if (pkt->stream_index == vs)
            frames++;
        av_packet_unref(pkt);
    }
    av_packet_free(&pkt);
    avformat_close_input(&ic);
    if (ret < 0 && ret != AVERROR_EOF) {
        char err[AV_ERROR_MAX_STRING_SIZE] = { 0 };
        av_strerror(ret, err, sizeof(err));
        set_error(GIF2MP4_E_INPUT, "read input failed: %s", err);
        return GIF2MP4_E_INPUT;
    }
    return frames;
}

/* ── 主转换 ──────────────────────────────────────────────────────── */

int gif2mp4_run(const gif2mp4_options *opt,
                const char *input_path, const char *output_path)
{
    const char *preset = "medium";
    char *tmp_path = NULL;
    char filter_desc[256];
    int fps, crf, cancelled = 0, ret_code = GIF2MP4_OK;
    int64_t total_frames = 0, decoded_frames = 0, out_frame_index = 0;
    int64_t skipped_packets = 0;
    int consecutive_errors = 0;
    double duration_sec = -1.0;

    /* libav 上下文（统一 cleanup 标签释放） */
    AVFormatContext *ic = NULL;
    const AVCodec *dec = NULL, *enc = NULL;
    AVCodecContext *dec_ctx = NULL, *enc_ctx = NULL;
    AVFilterGraph *graph = NULL;
    AVFilterContext *src = NULL, *sink = NULL;
    AVFilterInOut *outputs = NULL, *inputs = NULL;
    AVFrame *frame = NULL, *filt_frame = NULL;
    AVPacket *pkt = NULL, *enc_pkt = NULL;
    AVFormatContext *oc = NULL;
    AVStream *ost = NULL;
    AVRational sink_tb = { 0, 1 };
    int vs = -1;

    /* —— 参数校验 —— */
    if (!opt || !input_path || !output_path || !input_path[0] || !output_path[0])
        return set_error(GIF2MP4_E_ARG,
                         "invalid argument: paths must be non-empty and opt non-NULL");
    if (opt->struct_size != sizeof(gif2mp4_options))
        return set_error(GIF2MP4_E_ARG,
                         "options struct_size mismatch: got %u expect %zu "
                         "(recompile host against gif2mp4.h)",
                         (unsigned)opt->struct_size, sizeof(gif2mp4_options));
    if (opt->fps > 240)
        return set_error(GIF2MP4_E_ARG,
                         "invalid fps %d (allowed 1-240, or <=0 to follow source timing)",
                         opt->fps);
    if (opt->crf > 51)
        return set_error(GIF2MP4_E_ARG, "invalid crf %d (allowed 0-51)", opt->crf);
    if (opt->preset && opt->preset[0]) {
        const char *const *p;
        int found = 0;
        for (p = k_preset_whitelist; *p; p++)
            if (!strcmp(*p, opt->preset)) {
                found = 1;
                break;
            }
        if (!found)
            return set_error(GIF2MP4_E_ARG,
                             "invalid preset \"%s\" (whitelist: ultrafast..placebo)",
                             opt->preset);
        preset = opt->preset;
    }
    fps = opt->fps > 0 ? opt->fps : 0;
    crf = opt->crf >= 0 ? opt->crf : 23;
    /* opt->loop：MP4 无通用循环标志，no-op 保留位（见头文件） */

    install_log_adapter();
    if (!busy_try_acquire())
        return set_error(GIF2MP4_E_BUSY, "another conversion is already running");

    lib_log(GIF2MP4_LOG_INFO,
            "gif2mp4: start in=%s out=%s fps=%s crf=%d preset=%s",
            input_path, output_path, fps > 0 ? "fixed" : "source", crf, preset);

    /* —— 第一遍：精确总帧数 —— */
    {
        int64_t n = count_gif_frames(input_path, &duration_sec);
        if (n < 0) {
            ret_code = (int)n;
            goto cleanup;
        }
        if (n == 0) {
            ret_code = set_error(GIF2MP4_E_INPUT, "input has zero video frames: %s",
                                 input_path);
            goto cleanup;
        }
        total_frames = n;
    }

    /* —— 打开输入（第二遍） —— */
    {
        int ret = avformat_open_input(&ic, input_path, NULL, NULL);
        char err[AV_ERROR_MAX_STRING_SIZE] = { 0 };
        if (ret < 0) {
            av_strerror(ret, err, sizeof(err));
            ret_code = set_error(GIF2MP4_E_INPUT, "reopen input failed: %s", err);
            goto cleanup;
        }
        if (avformat_find_stream_info(ic, NULL) < 0) {
            ret_code = set_error(GIF2MP4_E_INPUT, "probe failed on second pass");
            goto cleanup;
        }
        vs = av_find_best_stream(ic, AVMEDIA_TYPE_VIDEO, -1, -1, &dec, 0);
        if (vs < 0) {
            ret_code = set_error(GIF2MP4_E_INPUT, "no decodable video stream");
            goto cleanup;
        }
    }
    dec_ctx = avcodec_alloc_context3(dec);
    if (!dec_ctx) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }
    if (avcodec_parameters_to_context(dec_ctx, ic->streams[vs]->codecpar) < 0) {
        ret_code = set_error(GIF2MP4_E_DECODE, "codec params to context failed");
        goto cleanup;
    }
    dec_ctx->framerate = ic->streams[vs]->avg_frame_rate;
    dec_ctx->time_base = ic->streams[vs]->time_base;
    if (avcodec_open2(dec_ctx, dec, NULL) < 0) {
        ret_code = set_error(GIF2MP4_E_DECODE, "open gif decoder failed");
        goto cleanup;
    }

    /* —— filter graph：[fps=N,]scale,format —— */
    graph = avfilter_graph_alloc();
    if (!graph) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }
    {
        char args[512];
        char fps_part[32] = "";
        if (fps > 0)
            snprintf(fps_part, sizeof(fps_part), "fps=%d,", fps);
        snprintf(args, sizeof(args),
                 "video_size=%dx%d:pix_fmt=%d:time_base=%d/%d:pixel_aspect=%d/%d",
                 dec_ctx->width, dec_ctx->height, dec_ctx->pix_fmt,
                 dec_ctx->time_base.num ? dec_ctx->time_base.num : 1,
                 dec_ctx->time_base.den ? dec_ctx->time_base.den : 1,
                 dec_ctx->sample_aspect_ratio.num ? dec_ctx->sample_aspect_ratio.num : 1,
                 dec_ctx->sample_aspect_ratio.den ? dec_ctx->sample_aspect_ratio.den : 1);
        src = avfilter_graph_alloc_filter(graph, avfilter_get_by_name("buffer"), "in");
        if (!src) {
            ret_code = set_error(GIF2MP4_E_INTERNAL, "alloc buffersrc failed");
            goto cleanup;
        }
        if (avfilter_init_str(src, args) < 0) {
            ret_code = set_error(GIF2MP4_E_DECODE,
                                 "init buffersrc failed (w=%d h=%d pix_fmt=%d)",
                                 dec_ctx->width, dec_ctx->height, dec_ctx->pix_fmt);
            goto cleanup;
        }
        sink = avfilter_graph_alloc_filter(graph, avfilter_get_by_name("buffersink"),
                                           "out");
        if (!sink) {
            ret_code = set_error(GIF2MP4_E_INTERNAL, "alloc buffersink failed");
            goto cleanup;
        }
        if (avfilter_init_str(sink, NULL) < 0) {
            ret_code = set_error(GIF2MP4_E_INTERNAL, "init buffersink failed");
            goto cleanup;
        }
        snprintf(filter_desc, sizeof(filter_desc),
                 "%sscale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p", fps_part);
    }
    outputs = avfilter_inout_alloc();
    inputs = avfilter_inout_alloc();
    if (!outputs || !inputs) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }
    outputs->name       = av_strdup("in");
    outputs->filter_ctx = src;
    outputs->pad_idx    = 0;
    outputs->next       = NULL;
    inputs->name        = av_strdup("out");
    inputs->filter_ctx  = sink;
    inputs->pad_idx     = 0;
    inputs->next        = NULL;
    if (avfilter_graph_parse_ptr(graph, filter_desc, &inputs, &outputs, NULL) < 0) {
        ret_code = set_error(GIF2MP4_E_DECODE, "parse filter graph failed: %s",
                             filter_desc);
        goto cleanup;
    }
    if (avfilter_graph_config(graph, NULL) < 0) {
        ret_code = set_error(GIF2MP4_E_DECODE, "configure filter graph failed");
        goto cleanup;
    }
    sink_tb = av_buffersink_get_time_base(sink);

    /* —— 编码器（libx264；crf/preset 是其私有 AVOption） —— */
    enc = avcodec_find_encoder_by_name("libx264");
    if (!enc) {
        ret_code = set_error(GIF2MP4_E_ENCODE,
                             "libx264 encoder not present in this build");
        goto cleanup;
    }
    enc_ctx = avcodec_alloc_context3(enc);
    if (!enc_ctx) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }
    enc_ctx->width      = av_buffersink_get_w(sink);
    enc_ctx->height     = av_buffersink_get_h(sink);
    enc_ctx->pix_fmt    = (enum AVPixelFormat)av_buffersink_get_format(sink);
    enc_ctx->sample_aspect_ratio = av_buffersink_get_sample_aspect_ratio(sink);
    enc_ctx->time_base  = fps > 0 ? (AVRational){ 1, fps } : sink_tb;
    enc_ctx->flags     |= AV_CODEC_FLAG_GLOBAL_HEADER; /* mp4 muxer 需要 */
    {
        char crf_str[8];
        snprintf(crf_str, sizeof(crf_str), "%d", crf);
        if (av_opt_set(enc_ctx->priv_data, "crf", crf_str, 0) < 0) {
            ret_code = set_error(GIF2MP4_E_ENCODE, "set crf=%d failed", crf);
            goto cleanup;
        }
        if (av_opt_set(enc_ctx->priv_data, "preset", preset, 0) < 0) {
            ret_code = set_error(GIF2MP4_E_ENCODE, "set preset=%s failed", preset);
            goto cleanup;
        }
    }
    if (avcodec_open2(enc_ctx, enc, NULL) < 0) {
        ret_code = set_error(GIF2MP4_E_ENCODE, "open libx264 encoder failed");
        goto cleanup;
    }

    /* —— 输出（临时文件 + faststart） —— */
    tmp_path = av_asprintf("%s.tmp", output_path);
    if (!tmp_path) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }
    /* 显式指定 mp4 格式：临时文件名以 .tmp 结尾，按扩展名猜格式必失败
     * （"Unable to choose an output format"）；指定格式名也与裁剪构建
     * 解耦——不管注册了几个 muxer 都确定性命中 */
    if (avformat_alloc_output_context2(&oc, NULL, "mp4", tmp_path) < 0) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "alloc mp4 muxer failed");
        goto cleanup;
    }
    ost = avformat_new_stream(oc, NULL);
    if (!ost) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "new output stream failed");
        goto cleanup;
    }
    if (avcodec_parameters_from_context(ost->codecpar, enc_ctx) < 0) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "copy encoder params failed");
        goto cleanup;
    }
    ost->time_base = enc_ctx->time_base;
    if (!(oc->oformat->flags & AVFMT_NOFILE) &&
        avio_open(&oc->pb, tmp_path, AVIO_FLAG_WRITE) < 0) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "open output file failed: %s", tmp_path);
        goto cleanup;
    }
    /* moov 前置（流式播放友好）；mp4 muxer 私有选项 */
    av_opt_set(oc->priv_data, "movflags", "+faststart", AV_OPT_SEARCH_CHILDREN);
    if (avformat_write_header(oc, NULL) < 0) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "write mp4 header failed");
        goto cleanup;
    }

    frame      = av_frame_alloc();
    filt_frame = av_frame_alloc();
    pkt        = av_packet_alloc();
    enc_pkt    = av_packet_alloc();
    if (!frame || !filt_frame || !pkt || !enc_pkt) {
        ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
        goto cleanup;
    }

    /* —— 解码 → 滤镜 → 编码 → 封装主循环 —— */
    while (!cancelled) {
        int ret = av_read_frame(ic, pkt);
        if (ret == AVERROR_EOF)
            break;
        if (ret < 0) {
            ret_code = set_error(GIF2MP4_E_INPUT, "read packet failed: %d", ret);
            goto cleanup;
        }
        if (pkt->stream_index != vs) {
            av_packet_unref(pkt);
            continue;
        }
        ret = avcodec_send_packet(dec_ctx, pkt);
        av_packet_unref(pkt);
        if (ret < 0 && ret != AVERROR(EAGAIN)) {
            /* 损坏流容忍（对齐 ffmpeg CLI 行为）：解码错误跳包继续——
             * gif.ski 等导出器常见中段截断/坏帧，CLI 报 Error submitting
             * packet 数百次后照样出片。仅 ENOMEM 或连续坏到阈值才放弃。
             * send 的 EAGAIN 理论上不发生（每轮先排干 receive），若发生
             * 也按跳包处理 */
            if (ret == AVERROR(ENOMEM)) {
                ret_code = set_error(GIF2MP4_E_INTERNAL, "out of memory");
                goto cleanup;
            }
            skipped_packets++;
            if (++consecutive_errors > 512) {
                ret_code = set_error(GIF2MP4_E_DECODE,
                                     "too many consecutive decode errors (%d)",
                                     consecutive_errors);
                goto cleanup;
            }
            lib_log(GIF2MP4_LOG_WARN,
                    "gif2mp4: skip packet after send error %d (skipped=%" PRId64 ")",
                    ret, skipped_packets);
            continue;
        }
        while (ret >= 0) {
            ret = avcodec_receive_frame(dec_ctx, frame);
            if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
                break;
            if (ret < 0) {
                /* 同上：坏帧跳过继续收（continue 会重试 receive） */
                skipped_packets++;
                if (++consecutive_errors > 512) {
                    ret_code = set_error(GIF2MP4_E_DECODE,
                                         "too many consecutive decode errors (%d)",
                                         consecutive_errors);
                    goto cleanup;
                }
                lib_log(GIF2MP4_LOG_WARN,
                        "gif2mp4: skip frame after receive error %d (skipped=%" PRId64 ")",
                        ret, skipped_packets);
                continue;
            }
            consecutive_errors = 0;
            decoded_frames++;
            if (av_buffersrc_add_frame_flags(src, frame,
                                             AV_BUFFERSRC_FLAG_KEEP_REF) < 0) {
                ret_code = set_error(GIF2MP4_E_DECODE, "feed filter graph failed");
                goto cleanup;
            }
            av_frame_unref(frame);

            /* 拉取滤镜输出（fps 滤镜可能一进多出/多进一出） */
            for (;;) {
                ret = av_buffersink_get_frame(sink, filt_frame);
                if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
                    break;
                if (ret < 0) {
                    ret_code = set_error(GIF2MP4_E_DECODE, "filter frame failed: %d",
                                         ret);
                    goto cleanup;
                }
                /* pts：CFR 用输出帧序号重写；VFR 换算 sink 时基 */
                if (fps > 0)
                    filt_frame->pts = out_frame_index;
                else
                    filt_frame->pts = av_rescale_q(filt_frame->pts, sink_tb,
                                                   enc_ctx->time_base);
                filt_frame->duration  = 1;
                filt_frame->time_base = enc_ctx->time_base;
                if (avcodec_send_frame(enc_ctx, filt_frame) < 0) {
                    ret_code = set_error(GIF2MP4_E_ENCODE,
                                         "send frame to encoder failed");
                    goto cleanup;
                }
                av_frame_unref(filt_frame);
                out_frame_index++;
            }

            /* —— 进度上报（节流：首帧/每 16 帧） —— */
            if (opt->on_progress &&
                (decoded_frames == 1 ||
                 (decoded_frames % PROGRESS_EVERY_FRAMES) == 0)) {
                gif2mp4_progress p;
                p.decoded_frames = decoded_frames;
                p.total_frames   = total_frames;
                p.percent        = total_frames > 0
                                       ? (double)decoded_frames * 100.0 /
                                             (double)total_frames
                                       : -1.0;
                p.out_time_sec   = (double)out_frame_index * enc_ctx->time_base.num /
                                   (double)enc_ctx->time_base.den;
                p.duration_sec   = duration_sec;
                if (opt->on_progress(opt->progress_userdata, &p) != 0) {
                    cancelled = 1;
                    lib_log(GIF2MP4_LOG_INFO,
                            "gif2mp4: cancel requested by progress callback");
                }
            }
        }
    }

    /* —— flush 三级流水（取消路径跳过投喂，仅排空编码器） —— */
    if (!cancelled) {
        avcodec_send_packet(dec_ctx, NULL);
        while (avcodec_receive_frame(dec_ctx, frame) >= 0) {
            decoded_frames++;
            if (av_buffersrc_add_frame_flags(src, frame,
                                             AV_BUFFERSRC_FLAG_KEEP_REF) < 0) {
                ret_code = set_error(GIF2MP4_E_DECODE,
                                     "feed filter graph failed (flush)");
                goto cleanup;
            }
            av_frame_unref(frame);
        }
        if (av_buffersrc_add_frame(src, NULL) < 0) { /* 源 EOF，驱动滤镜排空 */
            ret_code = set_error(GIF2MP4_E_DECODE, "flush filter graph failed");
            goto cleanup;
        }
        for (;;) {
            int ret = av_buffersink_get_frame(sink, filt_frame);
            if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
                break;
            if (ret < 0) {
                ret_code = set_error(GIF2MP4_E_DECODE, "filter frame failed (flush)");
                goto cleanup;
            }
            if (fps > 0)
                filt_frame->pts = out_frame_index;
            else
                filt_frame->pts = av_rescale_q(filt_frame->pts, sink_tb,
                                               enc_ctx->time_base);
            filt_frame->duration  = 1;
            filt_frame->time_base = enc_ctx->time_base;
            if (avcodec_send_frame(enc_ctx, filt_frame) < 0) {
                ret_code = set_error(GIF2MP4_E_ENCODE,
                                     "send frame to encoder failed (flush)");
                goto cleanup;
            }
            av_frame_unref(filt_frame);
            out_frame_index++;
        }
        avcodec_send_frame(enc_ctx, NULL);
    }

    /* —— 编码器排空（取消路径也走：不留半开状态） —— */
    for (;;) {
        int ret = avcodec_receive_packet(enc_ctx, enc_pkt);
        if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
            break;
        if (ret < 0) {
            ret_code = set_error(GIF2MP4_E_ENCODE, "receive packet failed: %d", ret);
            goto cleanup;
        }
        if (!cancelled) {
            av_packet_rescale_ts(enc_pkt, enc_ctx->time_base, ost->time_base);
            enc_pkt->stream_index = ost->index;
            if (av_interleaved_write_frame(oc, enc_pkt) < 0) {
                ret_code = set_error(GIF2MP4_E_OUTPUT, "write frame failed");
                goto cleanup;
            }
        }
        av_packet_unref(enc_pkt);
    }

    /* —— 末帧进度 100%（取消场景不报） —— */
    if (opt->on_progress && !cancelled && total_frames > 0) {
        gif2mp4_progress p;
        memset(&p, 0, sizeof(p));
        p.decoded_frames = decoded_frames;
        p.total_frames   = total_frames;
        p.percent        = 100.0;
        p.out_time_sec   = (double)out_frame_index * enc_ctx->time_base.num /
                           (double)enc_ctx->time_base.den;
        p.duration_sec   = duration_sec;
        (void)opt->on_progress(opt->progress_userdata, &p);
    }

    /* 零帧守卫：损坏流可能全部跳包——没有任何可编码内容时按输入问题报错 */
    if (decoded_frames == 0) {
        ret_code = set_error(GIF2MP4_E_INPUT,
                             "no decodable frames (input corrupt? skipped=%" PRId64 ")",
                             skipped_packets);
        goto cleanup;
    }

    if (cancelled) {
        set_error(GIF2MP4_E_CANCELLED, "cancelled by progress callback");
        ret_code = GIF2MP4_E_CANCELLED;
        goto cleanup;
    }

    /* —— trailer + 原子替换 —— */
    if (av_write_trailer(oc) < 0) {
        ret_code = set_error(GIF2MP4_E_OUTPUT, "write mp4 trailer failed");
        goto cleanup;
    }
    if (!(oc->oformat->flags & AVFMT_NOFILE))
        avio_closep(&oc->pb);
    if (platform_replace(tmp_path, output_path) != 0) {
        /* 替换失败：临时文件仍在，统一由 cleanup 删除 */
        ret_code = set_error(GIF2MP4_E_OUTPUT, "atomic replace failed: %s -> %s",
                             tmp_path, output_path);
        goto cleanup;
    }
    lib_log(GIF2MP4_LOG_INFO,
            "gif2mp4: done (%" PRId64 " frames in, %" PRId64 " frames out, %" PRId64 " skipped)",
            decoded_frames, out_frame_index, skipped_packets);

cleanup:
    /* 失败/取消：删临时件（目标路径可能是宿主的旧文件，绝不动它）。
     * 成功路径 tmp 已 rename 走，remove 报错忽略。 */
    if (oc) {
        if (oc->pb)
            avio_closep(&oc->pb);
        avformat_free_context(oc);
    }
    if (tmp_path && ret_code != GIF2MP4_OK)
        platform_remove(tmp_path);
    av_packet_free(&enc_pkt);
    av_packet_free(&pkt);
    av_frame_free(&filt_frame);
    av_frame_free(&frame);
    avfilter_inout_free(&inputs);
    avfilter_inout_free(&outputs);
    avfilter_graph_free(&graph);
    avcodec_free_context(&enc_ctx);
    avcodec_free_context(&dec_ctx);
    avformat_close_input(&ic);
    av_free(tmp_path);
    busy_release();
    return ret_code;
}

/* ── 位置式便捷封装 ──────────────────────────────────────────────── */

int gif2mp4(const char *input_path, const char *output_path,
            int fps, int crf, const char *preset, int loop)
{
    gif2mp4_options opt;
    memset(&opt, 0, sizeof(opt));
    opt.struct_size = (uint32_t)sizeof(gif2mp4_options);
    opt.fps    = fps;
    opt.crf    = crf;
    opt.preset = preset;
    opt.loop   = loop;
    return gif2mp4_run(&opt, input_path, output_path);
}
