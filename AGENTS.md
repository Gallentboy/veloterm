# AGENTS.md

## File Editing Rules（文件写入硬性规范）

- **严禁通过 Bash 写入文件**：包括但不限于 `python3 <<EOF`、`echo >`、`tee`、`sed -i`、shell 重定向等任何变体。
- 修改或创建文件**必须使用专用文件写入工具**（Write / Edit / MultiEdit），确保每次改动精确、可审计、可回滚。
- Bash 仅允许执行只读命令与构建/测试（如 git status、grep、cargo check、pnpm build）。

## Commands

- `pnpm run dev` - Start the dev server
- `pnpm run build` - Build the app for production
- `pnpm run preview` - Preview the production build locally

## Docs

- Rsbuild: https://rsbuild.rs/llms.txt
- Rspack: https://rspack.rs/llms.txt
