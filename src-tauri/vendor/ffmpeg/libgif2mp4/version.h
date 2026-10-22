/*
 * gif2mp4 库版本宏（ffmpeg 树内库惯例：major 变更 = ABI 破坏）。
 *
 * 本文件是 ffmpeg 树的一部分（veloterm vendor，构建接线见同目录
 * Makefile 与顶层 configure/Makefile 补丁）。
 *
 * GIF2MP4_ABI_VERSION 与 gif2mp4.h 的同名宏保持一致：major 位同步递增。
 */

#ifndef GIF2MP4_VERSION_H
#define GIF2MP4_VERSION_H

#define GIF2MP4_VERSION_MAJOR 1
#define GIF2MP4_VERSION_MINOR 0
#define GIF2MP4_VERSION_MICRO 0

#define GIF2MP4_VERSION_INT \
    ((GIF2MP4_VERSION_MAJOR << 16) | (GIF2MP4_VERSION_MINOR << 8) | GIF2MP4_VERSION_MICRO)

#define GIF2MP4_VERSION       "1.0.0"
#define GIF2MP4_BUILD         "1"

#endif /* GIF2MP4_VERSION_H */
