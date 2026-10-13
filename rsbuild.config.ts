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
