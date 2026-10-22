/*
 * gif2mp4 库主版本号（ffmpeg 树内库惯例：version_major.h 独立于 version.h）。
 *
 * 拆分的意义：ffbuild/library.mak 的 `lib$(NAME).version` 规则依赖
 * `version.h` + `version_major.h` 两个文件（见 ffbuild/library.mak:76）——
 * 缺 version_major.h 会让该规则永不满足、make 每次重算都判定需重建，
 * 表现为 `GEN libgif2mp4/libgif2mp4.version` 无限循环。
 *
 * major 变更 = ABI 破坏；与 gif2mp4.h 的 GIF2MP4_ABI_VERSION 同步递增。
 */

#ifndef GIF2MP4_VERSION_MAJOR_H
#define GIF2MP4_VERSION_MAJOR_H

#define GIF2MP4_VERSION_MAJOR 1

#endif /* GIF2MP4_VERSION_MAJOR_H */
