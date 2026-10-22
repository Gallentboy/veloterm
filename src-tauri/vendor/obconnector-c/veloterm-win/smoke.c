/* obconnector-c Windows 构建冒烟：静态链入 obclnt.lib（连接器对象，
 * 不含 OpenSSL——CMake 只在 Unix 的 .a 合并分支并入 ssl/crypto，Windows
 * 静态库需消费方自带），+ vcpkg 静态 ssl/crypto。
 * 跑通 mysql_library_init → mysql_init → mysql_close 生命周期：
 * WIN32 的初始化路径会做 WSAStartup（顺带验证 ws2_32 接线），并打印
 * 客户端版本串（验证 mariadb_version.h 生成与符号导出面）。不实际连
 * 服务器——冒烟只验证编译产物可链可跑，协议行为由 app 侧接线后回归。 */
#include <stdio.h>
#include <mysql.h>

int main(void) {
  if (mysql_library_init(0, NULL, NULL) != 0) {
    fprintf(stderr, "smoke: mysql_library_init failed\n");
    return 1;
  }
  printf("smoke: client %s (%lu)\n", mysql_get_client_info(),
         (unsigned long)mysql_get_client_version());

  MYSQL *mysql = mysql_init(NULL);
  if (!mysql) {
    fprintf(stderr, "smoke: mysql_init failed\n");
    return 1;
  }
  mysql_close(mysql);
  mysql_library_end();
  printf("smoke ok\n");
  return 0;
}
