/*
 * gif2mp4 版本函数（对齐树内库惯例：*_version / *_configuration / *_license）。
 *
 * 本文件是 ffmpeg 树的一部分（veloterm vendor）。
 */

#include "config.h"

#include "gif2mp4.h"
#include "version.h"

unsigned gif2mp4_library_version(void)
{
    return GIF2MP4_VERSION_INT;
}
const char *gif2mp4_library_configuration(void)
{
    return FFMPEG_CONFIGURATION;
}

const char *gif2mp4_library_license(void)
{
#define LICENSE_PREFIX "libgif2mp4 license: "
    return &LICENSE_PREFIX FFMPEG_LICENSE[sizeof(LICENSE_PREFIX) - 1];
}
