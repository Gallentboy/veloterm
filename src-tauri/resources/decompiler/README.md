# Java 反编译常驻 Worker 资源目录

本目录包含反编译器运行时依赖与常驻胶水包（均为跨平台通用的静态 JVM 字节码资产，全平台通用同一份，构建机/CI 零编译依赖）：

- `vineflower.jar`：官方 Vineflower 1.12.0 依赖（1.6 MB）
- `VeloDecompilerWorker.java`：常驻 Worker 源码（几十行，纯内存流式通信）
- `velo-worker.jar`：已编译好的 Worker 静态字节码包（6.8 KB）

## 手动更新说明
如果后续修改了 `VeloDecompilerWorker.java`，在具备 Java 17+ 的机器上手动重新编译即可（一次编译，全平台通用）：

```bash
# 1. 编译 class
javac -encoding UTF-8 -cp vineflower.jar VeloDecompilerWorker.java

# 2. 打包为 velo-worker.jar
jar -cfe velo-worker.jar com.veloterm.decompiler.VeloDecompilerWorker com/veloterm/decompiler/*.class

# 3. 清理生成的临时 class 目录
rm -rf com
```
