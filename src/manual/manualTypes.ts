export interface ManualSection {
  /** 唯一小节标识，如 "terminal-pty" */
  id: string;
  /** 功能点名称 */
  title: string;
  /** 简短徽章标签，如 "PTY" / "Core" / "Pro" */
  badge?: string;
  /** 检索关键字列表 */
  keywords?: string[];
  /** 详细使用说明 Markdown 文本（按功能概述、操作入口、步骤指南、注意事项四段式编写） */
  content: string;
}

export interface ManualChapter {
  /** 章节标识，对应一级业务模块 */
  id: string;
  /** 章节标题，对应系统核心菜单/模块名 */
  title: string;
  /** Lucide 图标标识 */
  icon: string;
  /** 包含的功能点清单 */
  sections: ManualSection[];
}
