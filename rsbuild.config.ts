import path from 'node:path';
import { defineConfig } from '@rsbuild/core';
import { pluginReact } from '@rsbuild/plugin-react';
import { pluginTailwindcss } from '@rsbuild/plugin-tailwindcss';

// 构建元数据注入（关于弹窗的“构建时间”行，参考 ZCode build-meta）
const BUILD_TIME = new Date().toISOString();

// Docs: https://rsbuild.rs/config/
export default defineConfig({
  plugins: [pluginReact(), pluginTailwindcss()],
  source: {
    define: {
      __BUILD_TIME__: JSON.stringify(BUILD_TIME),
    },
  },
  html: {
    title: 'VeloTerm',
  },
  tools: {
    cssLoader: {
      // CSS 里指向 public/ 静态资源的根相对 url()（如 /fonts/HarmonyOS_Sans_SC.ttf）
      // 交给运行时按站点根解析，不让 css-loader 当模块请求去解析（否则打包期报 Module not found）。
      url: {
        filter: (url: string) => !url.startsWith('/fonts/'),
      },
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 3000,
    strictPort: true,
  },
});
