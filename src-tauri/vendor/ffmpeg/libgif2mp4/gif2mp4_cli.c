/*
 * gif2mp4_cli — libgif2mp4.a 的命令行冒烟程序。
 *
 * 用法：
 *   ./gif2mp4_cli <input.gif> <output.mp4> [fps] [crf] [preset]
 *   省略的可选参数取库缺省（fps=0 跟随 GIF 时基 / crf=23 / preset=medium）；
 *   preset 传 "0" 也表示用缺省。
 *
 * 行为：注册日志回调 → 全部日志（含 av_log 重定向）打到 stdout；注册
 * 进度回调 → 逐次打印进度（库侧已按 16 帧节流）；结束打印返回码与
 * last_error（失败时）。退出码：0 成功，1 失败。
 *
 * 编译（在 ffmpeg 树根执行；链接序 = 依赖者在前，avutil 殿后，x264 压轴）：
 *
 *   cc -o libgif2mp4/gif2mp4_cli libgif2mp4/gif2mp4_cli.c \
 *     -I. -Ilibgif2mp4 \
 *     libgif2mp4/libgif2mp4.a \
 *     libavfilter/libavfilter.a \
 *     libswscale/libswscale.a \
 *     libavformat/libavformat.a \
 *     libavcodec/libavcodec.a \
 *     libswresample/libswresample.a \
 *     libavutil/libavutil.a \
 *     /opt/homebrew/Cellar/x264/r3222/lib/libx264.a \
 *     -lpthread -lm
 *
 * 若链接报 _CFxxx/_Obxxx 未定义（个别配置会带 CoreFoundation/iconv）：
 * 追加 `-framework CoreFoundation -liconv`。
 */

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gif2mp4.h"

static const char *log_tag(gif2mp4_log_level level)
{
    switch (level) {
    case GIF2MP4_LOG_ERROR:   return "ERROR";
    case GIF2MP4_LOG_WARN:    return "WARN ";
    case GIF2MP4_LOG_INFO:    return "INFO ";
    case GIF2MP4_LOG_VERBOSE: return "VERB ";
    case GIF2MP4_LOG_DEBUG:   return "DEBUG";
    }
    return "?    ";
}

/* 日志回调：stdout 直出（单行已去尾换行，这里补回） */
static void cli_log_cb(void *userdata, gif2mp4_log_level level, const char *message)
{
    (void)userdata;
    fprintf(stdout, "[gif2mp4][%s] %s\n", log_tag(level), message);
    fflush(stdout);
}

/* 进度回调：返回 0 = 不取消 */
static int cli_progress_cb(void *userdata, const gif2mp4_progress *p)
{
    (void)userdata;
    if (p->percent >= 0.0)
        fprintf(stdout,
                "[progress] %.1f%%  (%" PRId64 "/%" PRId64 " 帧, 输出 %.2fs / 总 %.2fs)\n",
                p->percent, p->decoded_frames, p->total_frames,
                p->out_time_sec, p->duration_sec);
    else
        fprintf(stdout, "[progress] 已解码 %" PRId64 " 帧（总帧数未知）\n",
                p->decoded_frames);
    fflush(stdout);
    return 0;
}

static int parse_int(const char *s, int *out)
{
    char *end = NULL;
    long v;
    if (!s || !s[0])
        return 0;
    v = strtol(s, &end, 10);
    if (!end || *end != '\0')
        return 0;
    *out = (int)v;
    return 1;
}

int main(int argc, char **argv)
{
    const char *in_path, *out_path;
    gif2mp4_options opt;
    int fps = 0, crf = -1, ret;
    const char *preset = NULL;

    if (argc < 3) {
        fprintf(stderr,
                "用法: %s <input.gif> <output.mp4> [fps] [crf] [preset]\n"
                "  fps    0 = 跟随 GIF 时基（缺省）；如 30/60 固定帧率\n"
                "  crf    <0 = 缺省 23（范围 0-51，越小质量越高）\n"
                "  preset 缺省 medium（ultrafast..placebo 十档）\n",
                argv[0]);
        return 1;
    }
    in_path  = argv[1];
    out_path = argv[2];
    if (argc > 3 && !parse_int(argv[3], &fps)) {
        fprintf(stderr, "fps 非法: %s\n", argv[3]);
        return 1;
    }
    if (argc > 4 && !parse_int(argv[4], &crf)) {
        fprintf(stderr, "crf 非法: %s\n", argv[4]);
        return 1;
    }
    if (argc > 5 && strcmp(argv[5], "0") != 0)
        preset = argv[5];

    fprintf(stdout, "gif2mp4_cli: ABI=%d\n", gif2mp4_abi_version());
    gif2mp4_set_log_callback(cli_log_cb, NULL, GIF2MP4_LOG_INFO);

    memset(&opt, 0, sizeof(opt));
    opt.struct_size  = (uint32_t)sizeof(gif2mp4_options);
    opt.fps          = fps;
    opt.crf          = crf;                 /* <0 → 库内取 23 */
    opt.preset       = preset;              /* NULL → 库内取 medium */
    opt.loop         = 0;                   /* no-op 保留位 */
    opt.on_progress  = cli_progress_cb;
    opt.progress_userdata = NULL;

    ret = gif2mp4_run(&opt, in_path, out_path);
    if (ret == GIF2MP4_OK) {
        fprintf(stdout, "结果: 成功 → %s\n", out_path);
        return 0;
    }
    fprintf(stdout, "结果: 失败 code=%d\n错误: %s\n", ret, gif2mp4_last_error());
    return 1;
}
