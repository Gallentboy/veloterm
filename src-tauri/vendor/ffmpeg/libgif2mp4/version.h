/*
 * gif2mp4 库版本宏（ffmpeg 树内库惯例：major 在 version_major.h，
 * 本文件只放 minor/micro 与派生宏）。
 *
 * 本文件是 ffmpeg 树的一部分（veloterm vendor，构建接线见同目录
 * Makefile 与顶层 configure/Makefile 补丁）。
 *
 * GIF2MP4_ABI_VERSION 与 gif2mp4.h 的同名宏保持一致：major 位同步递增。
 */

#ifndef GIF2MP4_VERSION_H
#define GIF2MP4_VERSION_H

/**
 * @file
 * gif2mp4 version macros
 */

#include "libavutil/version.h"

#include "version_major.h"

#define GIF2MP4_VERSION_MINOR 0
#define GIF2MP4_VERSION_MICRO 0

#define GIF2MP4_VERSION_INT  AV_VERSION_INT(GIF2MP4_VERSION_MAJOR, \
                                            GIF2MP4_VERSION_MINOR, \
                                            GIF2MP4_VERSION_MICRO)
#define GIF2MP4_VERSION      AV_VERSION(GIF2MP4_VERSION_MAJOR, \
                                        GIF2MP4_VERSION_MINOR, \
                                        GIF2MP4_VERSION_MICRO)
#define GIF2MP4_BUILD        GIF2MP4_VERSION_INT

#define GIF2MP4_IDENT        "G2M" AV_STRINGIFY(GIF2MP4_VERSION)

#endif /* GIF2MP4_VERSION_H */
