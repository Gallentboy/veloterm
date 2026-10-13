import { useEffect, useState } from 'react';
import { getVersion, getTauriVersion } from '@tauri-apps/api/app';
import pkg from '../../package.json';

export interface AppVersionInfo {
  /** 应用版本（Tauri getVersion，浏览器环境回退 package.json） */
  version: string;
  tauriVersion: string;
  /** AppleWebKit 构建号（UA 中解析，解析不到回退 WKWebView） */
  webkitVersion: string;
  reactVersion: string;
  isAppleSilicon: boolean;
}

function parseWebkitVersion(): string {
  const match = /AppleWebKit\/([\d.]+)/.exec(navigator.userAgent);
  return match?.[1] ?? 'WKWebView';
}

function parseReactVersion(): string {
  return (pkg.dependencies?.react ?? '').replace(/^\^/, '') || '—';
}

/** 应用版本/运行时信息（关于弹窗与设置页共用；package.json 是版本唯一来源）。 */
export function useAppVersion(): AppVersionInfo {
  const [info, setInfo] = useState<AppVersionInfo>(() => ({
    version: pkg.version,
    tauriVersion: '',
    webkitVersion: parseWebkitVersion(),
    reactVersion: parseReactVersion(),
    isAppleSilicon: false,
  }));

  useEffect(() => {
    let alive = true;
    void Promise.all([
      getVersion().catch(() => pkg.version),
      getTauriVersion().catch(() => ''),
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke<boolean>('is_apple_silicon'))
        .catch(() => false),
    ]).then(([version, tauriVersion, isAppleSilicon]) => {
      if (alive) {
        setInfo({
          version,
          tauriVersion,
          webkitVersion: parseWebkitVersion(),
          reactVersion: parseReactVersion(),
          isAppleSilicon,
        });
      }
    });
    return () => {
      alive = false;
    };
  }, []);

  return info;
}
