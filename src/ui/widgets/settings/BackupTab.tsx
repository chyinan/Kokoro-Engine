import React, { useState } from 'react';
import { motion } from 'framer-motion';
import { clsx } from 'clsx';
import { Download, Upload, Loader2, Check, AlertTriangle, Database, FileJson } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { save, open } from '@tauri-apps/plugin-dialog';
import { exportData, previewImport, importData } from '../../../lib/kokoro-bridge';
import type { ImportPreview, ImportOptions } from '../../../lib/kokoro-bridge';
import { sectionHeadingClasses } from '../../styles/settings-primitives';

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export const BackupTab: React.FC = () => {
    const { t } = useTranslation();

    // Export state
    const [exporting, setExporting] = useState(false);
    const [exportResult, setExportResult] = useState<{ size: string; stats: string } | null>(null);
    const [exportError, setExportError] = useState<string | null>(null);

    // Import state
    const [preview, setPreview] = useState<ImportPreview | null>(null);
    const [importing, setImporting] = useState(false);
    const [importDone, setImportDone] = useState<string | null>(null);
    const [importError, setImportError] = useState<string | null>(null);
    const [importFilePath, setImportFilePath] = useState<string | null>(null);

    // Import options
    const [importDb, setImportDb] = useState(true);
    const [importConfigs, setImportConfigs] = useState(true);
    const [conflictStrategy, setConflictStrategy] = useState<"skip" | "overwrite">("overwrite");

    const handleExport = async () => {
        setExporting(true);
        setExportResult(null);
        setExportError(null);
        try {
            const filePath = await save({
                defaultPath: `kokoro-backup-${new Date().toISOString().slice(0, 10)}.kokoro`,
                filters: [{ name: 'Kokoro Backup', extensions: ['kokoro'] }],
            });
            if (!filePath) { setExporting(false); return; }

            const result = await exportData(filePath);
            setExportResult({
                size: formatBytes(result.size_bytes),
                stats: t('settings.backup.export_stats', {
                    memories: result.stats.memories,
                    conversations: result.stats.conversations,
                    configs: result.stats.configs,
                }),
            });
        } catch (e: any) {
            setExportError(String(e));
        } finally {
            setExporting(false);
        }
    };

    const handleSelectImport = async () => {
        setPreview(null);
        setImportDone(null);
        setImportError(null);
        try {
            const filePath = await open({
                filters: [{ name: 'Kokoro Backup', extensions: ['kokoro'] }],
                multiple: false,
            });
            if (!filePath) return;
            const path = typeof filePath === 'string' ? filePath : filePath[0];
            setImportFilePath(path);
            const p = await previewImport(path);
            setPreview(p);
            setImportDb(p.has_database);
            setImportConfigs(p.has_configs);
        } catch (e: any) {
            setImportError(String(e));
        }
    };

    const handleImport = async () => {
        if (!importFilePath) return;
        setImporting(true);
        setImportError(null);
        setImportDone(null);
        try {
            const options: ImportOptions = {
                import_database: importDb,
                import_configs: importConfigs,
                conflict_strategy: conflictStrategy,
            };
            const result = await importData(importFilePath, options);
            setImportDone(t('settings.backup.import_stats', {
                memories: result.imported_memories,
                conversations: result.imported_conversations,
                configs: result.imported_configs,
            }));
            setPreview(null);
        } catch (e: any) {
            setImportError(String(e));
        } finally {
            setImporting(false);
        }
    };

    const toggleClasses = clsx(
        "relative w-9 h-5 rounded-full transition-colors cursor-pointer",
        "after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:w-4 after:h-4 after:rounded-full after:bg-white after:transition-transform"
    );

    return (
        <div className="space-y-6">
            {/* Export Section */}
            <div>
                <div className={clsx(sectionHeadingClasses, "mb-3")}>{t('settings.backup.export_title')}</div>
                <p className="text-xs text-[var(--color-text-muted)] mb-4">{t('settings.backup.export_desc')}</p>
                <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    onClick={handleExport}
                    disabled={exporting}
                    className={clsx(
                        "flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider",
                        "bg-[var(--color-accent)] text-black hover:bg-white transition-colors",
                        "disabled:opacity-50 disabled:cursor-not-allowed"
                    )}
                >
                    {exporting ? <Loader2 size={16} className="animate-spin" /> : <Download size={16} />}
                    {exporting ? t('settings.backup.exporting') : t('settings.backup.export_button')}
                </motion.button>
                {exportResult && (
                    <div className="mt-3 flex items-start gap-2 text-xs text-green-400">
                        <Check size={14} className="mt-0.5 shrink-0" />
                        <span>{t('settings.backup.export_success', { size: exportResult.size })} — {exportResult.stats}</span>
                    </div>
                )}
                {exportError && (
                    <div className="mt-3 flex items-start gap-2 text-xs text-red-400">
                        <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                        <span>{exportError}</span>
                    </div>
                )}
            </div>

            <div className="border-t border-[var(--color-border)]" />

            {/* Import Section */}
            <div>
                <div className={clsx(sectionHeadingClasses, "mb-3")}>{t('settings.backup.import_title')}</div>
                <p className="text-xs text-[var(--color-text-muted)] mb-4">{t('settings.backup.import_desc')}</p>
                <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    onClick={handleSelectImport}
                    className={clsx(
                        "flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider",
                        "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                    )}
                >
                    <Upload size={16} />
                    {t('settings.backup.import_button')}
                </motion.button>

                {/* Preview */}
                {preview && (
                    <div className="mt-4 p-4 rounded-lg bg-black/30 border border-[var(--color-border)] space-y-4">
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {t('settings.backup.preview_created', { date: new Date(preview.manifest.created_at).toLocaleString() })}
                            {' · v'}{preview.manifest.app_version}
                        </div>

                        {/* Stats */}
                        <div className="flex gap-4 text-xs">
                            {preview.has_database && (
                                <div className="flex items-center gap-1.5 text-[var(--color-text-secondary)]">
                                    <Database size={12} />
                                    <span>{preview.stats.memories} {t('settings.backup.memories')} · {preview.stats.conversations} {t('settings.backup.conversations')}</span>
                                </div>
                            )}
                            {preview.has_configs && (
                                <div className="flex items-center gap-1.5 text-[var(--color-text-secondary)]">
                                    <FileJson size={12} />
                                    <span>{preview.config_files.length} {t('settings.backup.config_files')}</span>
                                </div>
                            )}
                        </div>

                        {/* Options */}
                        <div className="space-y-2">
                            {preview.has_database && (
                                <label className="flex items-center gap-2 text-xs text-[var(--color-text-primary)] cursor-pointer">
                                    <input type="checkbox" checked={importDb} onChange={e => setImportDb(e.target.checked)}
                                        className={clsx(toggleClasses, importDb ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]")}
                                        style={{ appearance: 'auto' }}
                                    />
                                    {t('settings.backup.option_database')}
                                </label>
                            )}
                            {preview.has_configs && (
                                <label className="flex items-center gap-2 text-xs text-[var(--color-text-primary)] cursor-pointer">
                                    <input type="checkbox" checked={importConfigs} onChange={e => setImportConfigs(e.target.checked)}
                                        style={{ appearance: 'auto' }}
                                    />
                                    {t('settings.backup.option_configs')}
                                </label>
                            )}
                        </div>

                        {/* Conflict strategy */}
                        <div className="space-y-1">
                            <div className="text-xs text-[var(--color-text-muted)] font-semibold">{t('settings.backup.conflict_label')}</div>
                            <div className="flex gap-2">
                                {(["overwrite", "skip"] as const).map(s => (
                                    <button key={s} onClick={() => setConflictStrategy(s)}
                                        className={clsx(
                                            "px-3 py-1.5 rounded text-xs font-heading transition-colors",
                                            conflictStrategy === s
                                                ? "bg-[var(--color-accent)] text-black"
                                                : "bg-black/30 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
                                        )}
                                    >
                                        {t(`settings.backup.conflict_${s}`)}
                                    </button>
                                ))}
                            </div>
                        </div>

                        {/* Confirm */}
                        <motion.button
                            whileHover={{ scale: 1.02 }}
                            whileTap={{ scale: 0.98 }}
                            onClick={handleImport}
                            disabled={importing || (!importDb && !importConfigs)}
                            className={clsx(
                                "flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider",
                                "bg-[var(--color-accent)] text-black hover:bg-white transition-colors",
                                "disabled:opacity-50 disabled:cursor-not-allowed"
                            )}
                        >
                            {importing ? <Loader2 size={16} className="animate-spin" /> : <Upload size={16} />}
                            {importing ? t('settings.backup.importing') : t('settings.backup.confirm_import')}
                        </motion.button>
                    </div>
                )}

                {importDone && (
                    <div className="mt-3 flex items-start gap-2 text-xs text-green-400">
                        <Check size={14} className="mt-0.5 shrink-0" />
                        <span>{importDone}</span>
                    </div>
                )}
                {importError && (
                    <div className="mt-3 flex items-start gap-2 text-xs text-red-400">
                        <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                        <span>{importError}</span>
                    </div>
                )}
            </div>
        </div>
    );
};
