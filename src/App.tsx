import { LucideProvider } from 'lucide-react';
import { AppShell } from '@/components/AppShell';
import { EditorProvider } from '@/editor/EditorProvider';
import { HostsProvider } from '@/hosts/HostsProvider';
import { SftpProvider } from '@/services/SftpProvider';
import { TerminalProvider } from '@/terminal/TerminalProvider';
import { ThemeProvider } from '@/theme/ThemeProvider';

/** 与 ZCode 一致：根层统一注入 Lucide 默认描边 1.5，图标不回退到默认 2 而显粗；个别图标仍可显式覆盖。 */
const DEFAULT_LUCIDE_STROKE_WIDTH = 1.5;

const App = () => {
  return (
    <LucideProvider strokeWidth={DEFAULT_LUCIDE_STROKE_WIDTH}>
      <ThemeProvider>
        <HostsProvider>
          <TerminalProvider>
            <SftpProvider>
              <EditorProvider>
                <AppShell />
              </EditorProvider>
            </SftpProvider>
          </TerminalProvider>
        </HostsProvider>
      </ThemeProvider>
    </LucideProvider>
  );
};

export default App;
