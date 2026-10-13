import { createContext, useContext, type ReactNode } from 'react';
import { useHosts } from './useHosts';

/** 主机库共享上下文：主机页与分组页读写同一份状态。 */
const HostsContext = createContext<ReturnType<typeof useHosts> | null>(null);

export function HostsProvider({ children }: { children: ReactNode }) {
  const value = useHosts();
  return <HostsContext value={value}>{children}</HostsContext>;
}

export function useHostsStore(): ReturnType<typeof useHosts> {
  const ctx = useContext(HostsContext);
  if (!ctx) {
    throw new Error('useHostsStore 必须在 HostsProvider 内使用');
  }
  return ctx;
}
