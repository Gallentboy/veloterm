/*
 * gif2mp4 — GIF → MP4(H.264) 转码库（ffmpeg 树内第七库）
 *
 * 对外只暴露一组纯 C ABI（无异常、无 longjmp、绝不调用 exit()——
 * 库可能被长生命周期进程静态链入，任何错误都以返回码 + TLS 错误串
 * 出口，不终结进程）。
 *
 * 线程模型：
 * - gif2mp4_run() 为同步阻塞调用，宿主自行放工作线程；
 * - 进度回调发生在调用线程；日志回调可能来自 libavfilter/libx264
 *   的内部线程——回调实现必须可重入；
 * - 进程级单飞：转码进行中再次调用返回 GIF2MP4_E_BUSY；
 * - 转码进行中更换/注销日志回调属于不支持操作（指针与 userdata 的
 *   成对一致性无锁保证），但即使宿主违反约定也只会丢日志/错配
 *   userdata，绝不会崩溃（内部所有发射先判空、指针存取原子）。
 *
 * 回调内禁止事项：
 * - 禁止抛出 C++ 异常或 longjmp 穿越 C ABI 边界（未定义行为）；
 * - 禁止在回调内再调本库任何 API（死锁/重入）；
 * - 进度回调应只做置标志/轻量转发，阻塞会拖慢转码（但不死锁）。
 *
 * 未注册任何回调时的行为（防崩溃硬性规则）：
 * - 日志转发回 ffmpeg 原生默认回调（写 stderr）；本库自身事件静默，
 *   但错误详情永远可通过 gif2mp4_last_error() 取到；
 * - 无进度回调即无取消途径，转换跑到底。
 *
 * 输出原子性：全程写入 "<output>.tmp"，编码完整结束后才原子替换到
 * 目标路径；失败/取消/任何异常退出不残留半截 MP4。
 *
 * 许可注意：本库以 --enable-gpl + libx264 构建，组合产物整体按 GPL
 * 分发（商用集成前需项目方确认）。
 */

#ifndef GIF2MP4_GIF2MP4_H
#define GIF2MP4_GIF2MP4_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── ABI / 符号导出 ──────────────────────────────────────────────── */

#define GIF2MP4_ABI_VERSION 1

#if defined(GIF2MP4_DLL_EXPORTS) && defined(_WIN32)
#  define GIF2MP4_API __declspec(dllexport)
#elif defined(_WIN32)
#  define GIF2MP4_API /* 静态链入或导入端不加装饰 */
#elif defined(__GNUC__) && __GNUC__ >= 4
#  define GIF2MP4_API __attribute__((visibility("default")))
#else
#  define GIF2MP4_API
#endif

/* ── 错误码 ──────────────────────────────────────────────────────── */

#define GIF2MP4_OK              0  /* 成功 */
#define GIF2MP4_E_ARG          (-1) /* 参数非法（路径空/preset 不在白名单/
                                        crf 越界/fps 越界/struct_size 不符） */
#define GIF2MP4_E_INPUT        (-2) /* 打不开或读不出输入（不存在/非 GIF/损坏） */
#define GIF2MP4_E_DECODE       (-3) /* 解码失败 */
#define GIF2MP4_E_ENCODE       (-4) /* 编码失败 */
#define GIF2MP4_E_OUTPUT       (-5) /* 输出打开/封装/落盘失败 */
#define GIF2MP4_E_CANCELLED    (-6) /* 进度回调请求取消（现场已清理、无残留） */
#define GIF2MP4_E_BUSY         (-7) /* 已有转换在进行（进程级单飞） */
#define GIF2MP4_E_INTERNAL     (-8) /* 内部错误（内存不足等） */

/* ── 日志回调 ────────────────────────────────────────────────────── */

typedef enum {
    GIF2MP4_LOG_ERROR   = 0,
    GIF2MP4_LOG_WARN    = 1,
    GIF2MP4_LOG_INFO    = 2,
    GIF2MP4_LOG_VERBOSE = 3,
    GIF2MP4_LOG_DEBUG   = 4,
} gif2mp4_log_level;

/*
 * message 为已格式化的单行文本（无尾部换行）。
 * 线程注意：可能从 lib 内部线程并发调用，实现必须可重入（见文件头）。
 */
typedef void (*gif2mp4_log_cb)(void *userdata, gif2mp4_log_level level,
                               const char *message);

/*
 * 注册进程级日志回调；min_level 之下不再转发（libav 逐帧日志量大，
 * 静音到 WARN 是常用形态）。
 * cb 传 NULL = 注销：此后日志回到 ffmpeg 默认回调（stderr），绝不崩溃。
 * 建议在首次 gif2mp4_run 之前设置；转换进行中更换属不支持操作。
 */
GIF2MP4_API void gif2mp4_set_log_callback(gif2mp4_log_cb cb, void *userdata,
                                          gif2mp4_log_level min_level);

/* ── 进度回调与取消 ──────────────────────────────────────────────── */

typedef struct gif2mp4_progress {
    /* 已解码帧数（输入域计数，精确） */
    int64_t decoded_frames;
    /* 输入总帧数（两遍法第一遍精确计数）；取不到时为 0 */
    int64_t total_frames;
    /* 0-100；total 未知时为 -1（宿主显示不确定态进度条） */
    double percent;
    /* 已编码输出在输出时间轴上的位置（秒） */
    double out_time_sec;
    /* 输入总时长（秒）；未知为 -1 */
    double duration_sec;
} gif2mp4_progress;

/*
 * 返回非 0 表示请求取消：库在当前帧处理完毕后走清理路径并返回
 * GIF2MP4_E_CANCELLED（临时文件删除、无半截产物）。取消粒度为帧级。
 * NULL 回调 = 无进度上报、亦无取消途径。
 */
typedef int (*gif2mp4_progress_cb)(void *userdata,
                                   const gif2mp4_progress *progress);

/* ── 选项 ────────────────────────────────────────────────────────── */

typedef struct gif2mp4_options {
    /* 宿主必须填 sizeof(gif2mp4_options)——ABI 前向兼容判定用 */
    uint32_t struct_size;
    /*
     * 固定输出帧率（CFR）。GIF 自身时间戳常有 0 延迟帧/10ms 量化误差，
     * 固定 30/60 能让播放更顺滑。≤0 = 跟随 GIF 自身时基（VFR 直通）。
     * 取值范围 1..240。
     */
    int fps;
    /*
     * H.264 恒定速率因子（质量）。范围 0-51，越小质量越高、体积越大。
     * <0 取缺省 23。
     */
    int crf;
    /*
     * libx264 编码预设，白名单十档：
     * ultrafast/superfast/veryfast/faster/fast/medium/slow/slower/
     * veryslow/placebo。NULL 或 "" 取 "medium"。
     */
    const char *preset;
    /*
     * MP4 循环播放意愿位。MP4 容器无通用循环标志（多数播放器不认），
     * 当前实现为 no-op 保留位；真正的循环应由播放端控制。
     */
    int loop;
    /* 进度回调（NULL = 不要进度）。回调线程 = 调用线程。 */
    gif2mp4_progress_cb on_progress;
    void *progress_userdata;
} gif2mp4_options;

/* ── 主入口 ──────────────────────────────────────────────────────── */

/*
 * 阻塞执行转换：input_path(GIF) → output_path(MP4，已存在则覆盖)。
 * opt 不可为 NULL。返回 GIF2MP4_OK 或负数错误码；人类可读错误详情经
 * gif2mp4_last_error() 获取（线程本地存储，多线程各自独立）。
 * 并发：进程级单飞，占用期间返回 GIF2MP4_E_BUSY。
 */
GIF2MP4_API int gif2mp4_run(const gif2mp4_options *opt,
                            const char *input_path, const char *output_path);

/*
 * 位置式便捷封装（等价于全缺省 options）：
 * fps ≤0 跟随时基；crf <0 取 23；preset NULL/"" 取 medium；loop no-op；
 * 无进度回调。
 */
GIF2MP4_API int gif2mp4(const char *input_path, const char *output_path,
                        int fps, int crf, const char *preset, int loop);

/* 最近一次失败的错误详情（本线程；成功调用后不清空，仅失败路径写入） */
GIF2MP4_API const char *gif2mp4_last_error(void);

/* ABI 版本探测（宿主编译期与库运行期比对用），当前恒为
 * GIF2MP4_ABI_VERSION。 */
GIF2MP4_API int gif2mp4_abi_version(void);

/* ── 树内库惯例的版本三件套（宏见 version.h；对齐 swscale_version 等） ── */

GIF2MP4_API unsigned    gif2mp4_library_version(void);
GIF2MP4_API const char *gif2mp4_library_configuration(void);
GIF2MP4_API const char *gif2mp4_library_license(void);

#ifdef __cplusplus
}
#endif

#endif /* GIF2MP4_GIF2MP4_H */
