# obconnector-c 的 Windows 链接冒烟工程（由 build-tools.mjs 的 obclient 段
# 驱动，独立于上游根 CMakeLists——vendor 树零修补）。
# 入参（-D 传入）：
#   OBCLIENT_BUILD_DIR —— 上游构建树：取 libmariadb/$<CONFIG>/obclnt.lib 与
#                         生成的头文件（ma_config.h / mariadb_version.h 在
#                         <构建树>/include）
#   OPENSSL_ROOT_DIR   —— vcpkg x64/arm64-windows-static 安装根
cmake_minimum_required(VERSION 3.20)
project(obclient_smoke C)

add_executable(obclient-smoke smoke.c)
target_include_directories(obclient-smoke PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../include
  ${OBCLIENT_BUILD_DIR}/include)
target_link_libraries(obclient-smoke PRIVATE
  ${OBCLIENT_BUILD_DIR}/libmariadb/$<CONFIG>/obclnt.lib
  ${OPENSSL_ROOT_DIR}/lib/libssl.lib
  ${OPENSSL_ROOT_DIR}/lib/libcrypto.lib
  ws2_32 advapi32 crypt32 shlwapi)
