import i18n from 'i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import { initReactI18next } from 'react-i18next';
import { getDbKv, setDbKv } from '@/services/dbStorage';
import enUS from './locales/en-US';
import zhCN from './locales/zh-CN';

export const SUPPORTED_LOCALES = ['zh-CN', 'en-US'] as const;
export type Locale = (typeof SUPPORTED_LOCALES)[number];

export const LOCALE_STORAGE_KEY = 'veloterm-locale';

function normalizeLocale(lng: string): Locale {
  return lng.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';
}

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      'zh-CN': { translation: zhCN },
      'en-US': { translation: enUS },
    },
    supportedLngs: SUPPORTED_LOCALES,
    fallbackLng: 'zh-CN',
    interpolation: {
      // React 已做 XSS 转义
      escapeValue: false,
    },
    detection: {
      order: ['navigator'],
      caches: [],
      // navigator.language 形如 zh / zh-Hans-CN / en，统一归一到受支持的两个语言
      convertDetectedLanguage: normalizeLocale,
    },
  });

i18n.on('languageChanged', (lng) => {
  setDbKv(LOCALE_STORAGE_KEY, normalizeLocale(lng));
});

/**
 * 在 `bootstrapDbStorage()` 完成后同步数据库中的语言偏好。
 */
export function applyStoredLocale(): void {
  const saved = getDbKv(LOCALE_STORAGE_KEY);
  if (saved === 'zh-CN' || saved === 'en-US') {
    if (i18n.language !== saved) {
      void i18n.changeLanguage(saved);
    }
  }
}

export default i18n;
