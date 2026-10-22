# obconnector-c 的 Windows 链接冒烟工程（由 scripts/build-obclient.mjs 驱动，
# 独立于上游根 CMakeLists——vendor 树仅两处源码修补，其余全外参）。
# 入参（-D 传入）：
#   OBCLIENT_BUILD_DIR —— 上游构建树：取 libmariadb/$<CONFIG>/obclnt.lib 与
#                         生成的头文件（ma_config.h / mariadb_version.h 在
#                         <构建树>/include）
#   OPENSSL_ROOT_DIR   —— vcpkg x64/arm64-windows-static 安装根
cmake_minimum_required(VERSION 3.20)
project(obclient_smoke C)

add_executable(obclient-smoke smoke.c)
# 运行时库对齐 /MT：上游把 MSVC 默认 /MD 全量替换为 /MT（vcpkg 静态 ssl/crypto
# 也是 /MT），冒烟默认 /MD 会 LNK4098 双 CRT 混链
set_property(TARGET obclient-smoke PROPERTY MSVC_RUNTIME_LIBRARY MultiThreaded)

target_include_directories(obclient-smoke PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../include
  ${OBCLIENT_BUILD_DIR}/include)
target_link_libraries(obclient-smoke PRIVATE
  ${OBCLIENT_BUILD_DIR}/libmariadb/$<CONFIG>/obclnt.lib
  ${OPENSSL_ROOT_DIR}/lib/libssl.lib
  ${OPENSSL_ROOT_DIR}/lib/libcrypto.lib
  # 上游 SYSTEM_LIBS（ws2_32 advapi32 kernel32 shlwapi crypt32）之外，静态
  # 插件（caching_sha2/ob_sm3，plugins/auth 的 CRYPT_LIBS）引入 bcrypt
  # （CNG：win_crypt.c 的 ma_hash_new 与 SM3/caching_sha2 的加密原语）
  ws2_32 advapi32 crypt32 shlwapi bcrypt)
