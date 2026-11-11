package com.veloterm.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.Fernflower;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;

import java.io.BufferedReader;
import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.Collections;
import java.util.HashMap;
import java.util.Map;
import java.util.jar.Manifest;

/**
 * Veloterm Java 反编译常驻守护 Worker：
 * - 进程常驻在后台，通过 stdin / stdout 与 Rust 保持长连接单行文本交互
 * - 纯内存流转：接收 Base64 编码的单个 Class 字节流，调用 Vineflower 引擎微秒级反编译，回传 Java 源码
 * - 零磁盘 I/O，不写任何临时文件；单次反编译后立即清理 Context，杜绝内存泄漏
 */
public class VeloDecompilerWorker {

    public static void main(String[] args) {
        // 设置标准输出为 UTF-8 无缓冲
        System.out.println("READY");
        System.out.flush();

        BufferedReader reader = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;

        try {
            while ((line = reader.readLine()) != null) {
                line = line.trim();
                if (line.isEmpty()) {
                    continue;
                }
                if ("PING".equals(line)) {
                    System.out.println("PONG");
                    System.out.flush();
                    continue;
                }
                if ("EXIT".equals(line)) {
                    break;
                }

                // 命令格式: REQ \t <reqId> \t <className> \t <base64ClassBytes>
                if (line.startsWith("REQ\t")) {
                    handleDecompileRequest(line.substring(4));
                } else {
                    System.out.println("ERR\tUNKNOWN_COMMAND");
                    System.out.flush();
                }
            }
        } catch (Throwable t) {
            System.err.println("[VeloDecompilerWorker] Fatal error: " + t.getMessage());
            t.printStackTrace(System.err);
        }
    }

    private static void handleDecompileRequest(String payload) {
        // 解析 <reqId> \t <className> \t <base64ClassBytes>
        int tab1 = payload.indexOf('\t');
        if (tab1 < 0) {
            System.out.println("RES\tunknown\tERR\t" + toBase64("Invalid payload format (missing tab1)"));
            System.out.flush();
            return;
        }
        String reqId = payload.substring(0, tab1);

        int tab2 = payload.indexOf('\t', tab1 + 1);
        if (tab2 < 0) {
            System.out.println("RES\t" + reqId + "\tERR\t" + toBase64("Invalid payload format (missing tab2)"));
            System.out.flush();
            return;
        }
        String className = payload.substring(tab1 + 1, tab2);
        String base64Bytes = payload.substring(tab2 + 1);

        try {
            byte[] classBytes = Base64.getDecoder().decode(base64Bytes);
            String source = decompileSingleClass(className, classBytes);
            if (source == null || source.trim().isEmpty()) {
                System.out.println("RES\t" + reqId + "\tERR\t" + toBase64("Decompiler produced empty source"));
            } else {
                System.out.println("RES\t" + reqId + "\tOK\t" + toBase64(source));
            }
        } catch (Throwable e) {
            String errDetail = e.getClass().getSimpleName() + ": " + (e.getMessage() != null ? e.getMessage() : "unknown");
            System.out.println("RES\t" + reqId + "\tERR\t" + toBase64(errDetail));
        }
        System.out.flush();
    }

    private static String decompileSingleClass(String className, byte[] classBytes) {
        final String[] resultHolder = new String[1];

        // 规范化内部类名（如 com/example/Foo 或 Foo）
        String entryName = className.endsWith(".class") ? className.substring(0, className.length() - 6) : className;

        SingleClassContextSource source = new SingleClassContextSource(entryName, classBytes, resultHolder);

        Map<String, Object> options = new HashMap<>();
        options.put(IFernflowerPreferences.REMOVE_SYNTHETIC, "1");
        options.put(IFernflowerPreferences.DECOMPILE_GENERIC_SIGNATURES, "1");
        options.put(IFernflowerPreferences.DECOMPILE_INNER, "1");
        options.put(IFernflowerPreferences.BANNER, "");
        // 关键配置：纯内存单类分析，禁用对外部 JRT 运行时的物理扫描，保证极速
        options.put(IFernflowerPreferences.INCLUDE_JAVA_RUNTIME, "0");

        Fernflower fernflower = new Fernflower(new NullResultSaver(), options, new NullLogger());
        try {
            fernflower.addSource(source);
            fernflower.decompileContext();
        } finally {
            fernflower.clearContext();
            DecompilerContext.setCurrentContext(null);
        }

        return resultHolder[0];
    }

    private static String toBase64(String text) {
        return Base64.getEncoder().encodeToString(text.getBytes(StandardCharsets.UTF_8));
    }

    /**
     * 单类纯内存 ContextSource 实现
     */
    private static class SingleClassContextSource implements IContextSource {
        private final String className;
        private final byte[] classBytes;
        private final String[] resultHolder;

        public SingleClassContextSource(String className, byte[] classBytes, String[] resultHolder) {
            this.className = className;
            this.classBytes = classBytes;
            this.resultHolder = resultHolder;
        }

        @Override
        public String getName() {
            return "memory:" + className;
        }

        @Override
        public Entries getEntries() {
            return new Entries(
                Collections.singletonList(Entry.atBase(className)),
                Collections.emptyList(),
                Collections.emptyList()
            );
        }

        @Override
        public InputStream getInputStream(String resource) {
            if (resource != null && (resource.equals(className + ".class") || resource.equals(className))) {
                return new ByteArrayInputStream(classBytes);
            }
            return null;
        }

        @Override
        public IOutputSink createOutputSink(IResultSaver saver) {
            return new IOutputSink() {
                @Override
                public void begin() {}

                @Override
                public void acceptClass(String qualifiedName, String fileName, String content, int[] mapping) {
                    resultHolder[0] = content;
                }

                @Override
                public void acceptDirectory(String directory) {}

                @Override
                public void acceptOther(String path) {}

                @Override
                public void close() throws IOException {}
            };
        }
    }

    private static class NullResultSaver implements IResultSaver {
        @Override
        public void saveFolder(String path) {}
        @Override
        public void copyFile(String source, String path, String entryName) {}
        @Override
        public void saveClassFile(String path, String qualifiedName, String entryName, String content, int[] mapping) {}
        @Override
        public void createArchive(String path, String archiveName, Manifest manifest) {}
        @Override
        public void saveDirEntry(String path, String archiveName, String entryName) {}
        @Override
        public void copyEntry(String source, String path, String archiveName, String entry) {}
        @Override
        public void saveClassEntry(String path, String archiveName, String qualifiedName, String entryName, String content) {}
        @Override
        public void closeArchive(String path, String archiveName) {}
    }

    private static class NullLogger extends IFernflowerLogger {
        @Override
        public void writeMessage(String message, Severity severity) {}
        @Override
        public void writeMessage(String message, Severity severity, Throwable t) {}
    }
}
