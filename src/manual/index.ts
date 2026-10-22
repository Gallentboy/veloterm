import i18n from '@/i18n';
import { manualChaptersZh } from './manualDataZh';
import { manualChaptersEn } from './manualDataEn';
import type { ManualChapter, ManualSection } from './manualTypes';

export * from './manualTypes';

/** 获取当前系统语言对应全量手册章节列表 */
export function getManualChapters(lang?: string): ManualChapter[] {
  const currentLang = lang ?? i18n.language ?? 'zh-CN';
  return currentLang === 'en-US' ? manualChaptersEn : manualChaptersZh;
}

/** 扁平化所有小节，方便计算上一节 / 下一节 */
export interface FlatSectionItem {
  chapter: ManualChapter;
  section: ManualSection;
  globalIndex: number;
}

export function flattenManualSections(chapters: ManualChapter[]): FlatSectionItem[] {
  const flat: FlatSectionItem[] = [];
  let index = 0;
  for (const chapter of chapters) {
    for (const section of chapter.sections) {
      flat.push({
        chapter,
        section,
        globalIndex: index++,
      });
    }
  }
  return flat;
}
