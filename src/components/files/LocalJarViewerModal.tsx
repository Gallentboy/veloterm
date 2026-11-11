import { memo } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/ui/modal';
import { JarPreview } from '@/components/files/JarPreview';

export interface LocalJarViewerModalProps {
  filePath: string;
  fileName: string;
  isOpen: boolean;
  onClose: () => void;
}

export const LocalJarViewerModal = memo(function LocalJarViewerModal({
  filePath,
  fileName,
  isOpen,
  onClose,
}: LocalJarViewerModalProps) {
  const { t } = useTranslation();

  if (!isOpen) return null;

  return (
    <Modal
      title={`${fileName} — ${t('files.previewJar', { defaultValue: 'JAR 结构透视与反编译' })}`}
      onClose={onClose}
      className="h-[88vh] w-[92vw] max-w-7xl flex flex-col p-0 overflow-hidden rounded-xl border border-border"
    >
      <div className="flex-1 overflow-hidden h-full">
        <JarPreview filePath={filePath} fileName={fileName} className="h-full" />
      </div>
    </Modal>
  );
});
