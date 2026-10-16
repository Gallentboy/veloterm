
/***************************************************************************
 * nmap_winconfig.h -- Since the Windows port is currently eschewing       *
 * autoconf-style configure scripts, nmap_winconfig.h contains the         *
 * platform-specific definitions for Windows and is used as a replacement  *
 * for config.h                                                            *
 *                                                                         *
 ***********************IMPORTANT NMAP LICENSE TERMS************************
 *
 * The Nmap Security Scanner is (C) 1996-2026 Nmap Software LLC ("The Nmap
 * Project"). Nmap is also a registered trademark of the Nmap Project.
 *
 * This program is distributed under the terms of the Nmap Public Source
 * License (NPSL). The exact license text applying to a particular Nmap
 * release or source code control revision is contained in the LICENSE
 * file distributed with that version of Nmap or source code control
 * revision. More Nmap copyright/legal information is available from
 * https://nmap.org/book/man-legal.html, and further information on the
 * NPSL license itself can be found at https://nmap.org/npsl/ . This
 * header summarizes some key points from the Nmap license, but is no
 * substitute for the actual license text.
 *
 * Nmap is generally free for end users to download and use themselves,
 * including commercial use. It is available from https://nmap.org.
 *
 * The Nmap license generally prohibits companies from using and
 * redistributing Nmap in commercial products, but we sell a special Nmap
 * OEM Edition with a more permissive license and special features for
 * this purpose. See https://nmap.org/oem/
 *
 * If you have received a written Nmap license agreement or contract
 * stating terms other than these (such as an Nmap OEM license), you may
 * choose to use and redistribute Nmap under those terms instead.
 *
 * The official Nmap Windows builds include the Npcap software
 * (https://npcap.com) for packet capture and transmission. It is under
 * separate license terms which forbid redistribution without special
 * permission. So the official Nmap Windows builds may not be redistributed
 * without special permission (such as an Nmap OEM license).
 *
 * Source is provided to this software because we believe users have a
 * right to know exactly what a program is going to do before they run it.
 * This also allows you to audit the software for security holes.
 *
 * Source code also allows you to port Nmap to new platforms, fix bugs, and
 * add new features. You are highly encouraged to submit your changes as a
 * Github PR or by email to the dev@nmap.org mailing list for possible
 * incorporation into the main distribution. Unless you specify otherwise, it
 * is understood that you are offering us very broad rights to use your
 * submissions as described in the Nmap Public Source License Contributor
 * Agreement. This is important because we fund the project by selling licenses
 * with various terms, and also because the inability to relicense code has
 * caused devastating problems for other Free Software projects (such as KDE
 * and NASM).
 *
 * The free version of Nmap is distributed in the hope that it will be
 * useful, but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. Warranties,
 * indemnification and commercial support are all available through the
 * Npcap OEM program--see https://nmap.org/oem/
 *
 ***************************************************************************/

/* $Id$ */

#ifndef NMAP_WINCONFIG_H
#define NMAP_WINCONFIG_H
/* Without this, Windows will give us all sorts of crap about using functions
   like strcpy() even if they are done safely */
#define _CRT_SECURE_NO_DEPRECATE 1
/* VeloTerm patch：上游写死 i686（官方 Windows 构建只有 Win32）。本工程出
 * x64 DLL，按实际架构区分——仅影响 `nmap --version` 的 Platform 展示与
 * OS 指纹匹配字符串，无功能差异。 */
#if defined(_M_X64) || defined(__x86_64__)
#define NMAP_PLATFORM "x86_64-pc-windows-windows"
#else
#define NMAP_PLATFORM "i686-pc-windows-windows"
#endif

#define HAVE_OPENSSL 1
/* VeloTerm patch：不集成 libssh2（其需单独 CMake 构建 + OpenSSL 集成，
 * 成本高于收益；NSE 的 ssh 相关脚本会报 unavailable，其余不受影响）。
 * nmap.cc 用 `#if HAVE_LIBSSH2`、nse_main.cc 用 `#ifdef HAVE_LIBSSH2`，
 * 不定义即自动跳过；nse_libssh2.cc 亦不编入（见 veloterm-win/CMakeLists.txt）。 */
/* #define HAVE_LIBSSH2 1 */
#define HAVE_LIBZ 1
/* Since MSVC 2010, stdint.h is included as part of C99 compatibility */
#define HAVE_STDINT_H 1

#define LUA_INCLUDED 1
#undef PCAP_INCLUDED
#define DNET_INCLUDED 1
#define HAVE_DNET_LIB_VERSION 1
#define PCRE_INCLUDED 1
/* VeloTerm patch：同 HAVE_LIBSSH2（LIBSSH2_INCLUDED 仅在 HAVE_LIBSSH2 的
 * 分支里被 nmap.cc 使用，一并去掉避免误导） */
/* #define LIBSSH2_INCLUDED 1 */
#define ZLIB_INCLUDED 1

#endif /* NMAP_WINCONFIG_H */

