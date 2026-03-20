import React, { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { clsx } from 'clsx';
import { Download, Upload, Loader2, Check, AlertTriangle, Database, FileJson, Clock, FolderOpen, Trash2, Play } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { save, open } from '@tauri-apps/plugin-dialog';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { relaunch } from '@tauri-apps/plugin-process';
import { exportData, previewImport, importData, getAutoBackupConfig, saveAutoBackupConfig, runAutoBackupNow } from '../../../lib/kokoro-bridge';
import type { ImportPreview, AutoBackupConfig } from '../../../lib/kokoro-bridge';
import { characterDb } from '../../../lib/db';
import type { CharacterProfile } from '../../../lib/db';
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

    // Auto backup state
    const [autoBackup, setAutoBackup] = useState<AutoBackupConfig>({
        enabled: false,
        backup_dir: '',
        interval_days: 1,
        auto_cleanup: false,
        keep_days: 30,
    });
    const [autoBackupSaved, setAutoBackupSaved] = useState(false);
    const [autoBackupError, setAutoBackupError] = useState<string | null>(null);
    const [runningNow, setRunningNow] = useState(false);
    const [runNowResult, setRunNowResult] = useState<string | null>(null);

    useEffect(() => {
        getAutoBackupConfig().then(setAutoBackup).catch(() => {});
    }, []);

    const handleSaveAutoBackup = async () => {
        setAutoBackupError(null);
        setAutoBackupSaved(false);
        try {
            await saveAutoBackupConfig(autoBackup);
            setAutoBackupSaved(true);
            setTimeout(() => setAutoBackupSaved(false), 2000);
        } catch (e: any) {
            setAutoBackupError(String(e));
        }
    };

    const handlePickDir = async () => {
        const dir = await openDialog({ directory: true, multiple: false });
        if (dir && typeof dir === 'string') {
            setAutoBackup(prev => ({ ...prev, backup_dir: dir }));
        }
    };

    const handleRunNow = async () => {
        setRunningNow(true);
        setRunNowResult(null);
        setAutoBackupError(null);
        try {
            const path = await runAutoBackupNow();
            setRunNowResult(path);
        } catch (e: any) {
            setAutoBackupError(String(e));
        } finally {
            setRunningNow(false);
        }
    };

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

            // 序列化角色数据（头像 Blob 转 base64）+ 用户资料
            const chars = await characterDb.getAll();
            const charsSerializable = await Promise.all(chars.map(async (c) => {
                if (c.avatarBlob) {
                    const buf = await c.avatarBlob.arrayBuffer();
                    const b64 = btoa(String.fromCharCode(...new Uint8Array(buf)));
                    return { ...c, avatarBlob: undefined, avatarB64: b64 };
                }
                return { ...c, avatarBlob: undefined };
            }));
            const payload = {
                characters: charsSerializable,
                activeCharacterId: localStorage.getItem('kokoro_active_character_id'),
                userName: localStorage.getItem('kokoro_user_name'),
                userPersona: localStorage.getItem('kokoro_user_persona'),
                userLanguage: localStorage.getItem('kokoro_user_language'),
                responseLanguage: localStorage.getItem('kokoro_response_language'),
                voiceInterrupt: localStorage.getItem('kokoro_voice_interrupt'),
            };
            const charactersJson = JSON.stringify(payload);

            const result = await exportData(filePath, charactersJson);
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
            // Phase 1: 先恢复角色到 IndexedDB，拿到新 ID
            let targetCharacterId: string | undefined;
            let payload: any = null;

            // 先调用 importData 不带 target_character_id，拿到 characters_json
            const firstPass = await importData(importFilePath, {
                import_database: false,
                import_configs: false,
                conflict_strategy: conflictStrategy,
            });

            if (firstPass.characters_json && importDb) {
                try {
                    payload = JSON.parse(firstPass.characters_json);
                    const chars: (Omit<CharacterProfile, 'avatarBlob'> & { avatarB64?: string })[] =
                        payload.characters ?? payload; // 兼容旧格式

                    // 恢复用户资料 localStorage
                    if (payload.userName != null) localStorage.setItem('kokoro_user_name', payload.userName);
                    if (payload.userPersona != null) localStorage.setItem('kokoro_user_persona', payload.userPersona);
                    if (payload.userLanguage != null) localStorage.setItem('kokoro_user_language', payload.userLanguage);
                    if (payload.responseLanguage != null) localStorage.setItem('kokoro_response_language', payload.responseLanguage);
                    if (payload.voiceInterrupt != null) localStorage.setItem('kokoro_voice_interrupt', payload.voiceInterrupt);

                    // 先写入所有备份角色，全部成功后再删旧角色，避免中途失败导致数据损坏
                    const newIds: number[] = [];
                    for (const c of chars) {
                        let avatarBlob: Blob | undefined;
                        if (c.avatarB64) {
                            const bytes = Uint8Array.from(atob(c.avatarB64), ch => ch.charCodeAt(0));
                            avatarBlob = new Blob([bytes]);
                        }
                        const { avatarB64, id: _oldId, ...rest } = c;
                        const newId = await characterDb.add({ ...rest, avatarBlob });
                        newIds.push(newId);
                    }
                    // 所有新角色写入成功，再清空旧角色
                    const existing = await characterDb.getAll();
                    for (const c of existing) {
                        if (c.id !== undefined && !newIds.includes(c.id)) {
                            await characterDb.remove(c.id);
                        }
                    }
                    // stableId is preserved in rest, so use it directly
                    const oldActiveId = payload.activeCharacterId;
                    console.log('[Backup] oldActiveId (stableId):', oldActiveId);
                    targetCharacterId = oldActiveId || undefined;
                    // Fall back to first restored character's stableId
                    if (!targetCharacterId) {
                        const allRestored = await characterDb.getAll();
                        targetCharacterId = allRestored[0]?.stableId;
                    }
                    console.log('[Backup] targetCharacterId resolved to:', targetCharacterId);
                    if (targetCharacterId) {
                        localStorage.setItem('kokoro_active_character_id', targetCharacterId);
                    }
                } catch (e) {
                    console.error('[Backup] Failed to restore characters:', e);
                }
            }

            // 若仍未确定目标角色，回退到当前 localStorage 里的活跃角色
            if (!targetCharacterId) {
                targetCharacterId = localStorage.getItem('kokoro_active_character_id') ?? undefined;
            }

            // Phase 2: 用正确的 target_character_id 导入数据库和配置
            console.log('[Backup] Phase 2 importData options:', {
                import_database: importDb,
                import_configs: importConfigs,
                conflict_strategy: conflictStrategy,
                target_character_id: targetCharacterId,
            });
            const result = await importData(importFilePath, {
                import_database: importDb,
                import_configs: importConfigs,
                conflict_strategy: conflictStrategy,
                target_character_id: targetCharacterId,
            });
            console.log('[Backup] Phase 2 result:', result);

            const debugInfo = [
                `oldActiveId: ${payload?.activeCharacterId ?? 'n/a'}`,
                `targetCharacterId: ${targetCharacterId ?? 'undefined'}`,
                ...(result.debug_log ?? []),
            ].join('\n');
            setImportDone(
                t('settings.backup.import_stats', {
                    memories: result.imported_memories,
                    conversations: result.imported_conversations,
                    configs: result.imported_configs,
                }) + '\n\n[debug]\n' + debugInfo
            );
            setPreview(null);
            setTimeout(() => relaunch(), 1500);
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
            {/* Auto Backup Section */}
            <div>
                <div className={clsx(sectionHeadingClasses, "mb-3")}>{t('settings.backup.auto_title')}</div>
                <p className="text-xs text-[var(--color-text-muted)] mb-4">{t('settings.backup.auto_desc')}</p>

                <div className="space-y-3">
                    {/* Enable toggle */}
                    <label className="flex items-center gap-2 text-xs text-[var(--color-text-primary)] cursor-pointer">
                        <input type="checkbox" checked={autoBackup.enabled}
                            onChange={e => setAutoBackup(prev => ({ ...prev, enabled: e.target.checked }))}
                            className={clsx(toggleClasses, autoBackup.enabled ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]")}
                            style={{ appearance: 'none' }}
                        />
                        {t('settings.backup.auto_enable')}
                    </label>

                    {autoBackup.enabled && (
                        <div className="space-y-3 pl-1">
                            {/* Backup dir */}
                            <div>
                                <div className="text-xs text-[var(--color-text-muted)] mb-1">{t('settings.backup.auto_dir')}</div>
                                <div className="flex gap-2 items-center">
                                    <input
                                        type="text"
                                        readOnly
                                        value={autoBackup.backup_dir}
                                        placeholder={t('settings.backup.auto_dir_placeholder')}
                                        className="flex-1 px-3 py-1.5 rounded text-xs bg-black/30 border border-[var(--color-border)] text-[var(--color-text-secondary)] cursor-default"
                                    />
                                    <button onClick={handlePickDir}
                                        className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs border border-[var(--color-border)] text-[var(--color-text-muted)] hover:text-[var(--color-accent)] hover:border-[var(--color-accent)] transition-colors">
                                        <FolderOpen size={12} />
                                        {t('settings.backup.auto_dir_pick')}
                                    </button>
                                </div>
                            </div>

                            {/* Interval */}
                            <div>
                                <div className="text-xs text-[var(--color-text-muted)] mb-1">{t('settings.backup.auto_interval')}</div>
                                <div className="flex gap-1.5">
                                    {[1, 2, 3, 7].map(d => (
                                        <button key={d} onClick={() => setAutoBackup(prev => ({ ...prev, interval_days: d }))}
                                            className={clsx(
                                                "px-3 py-1.5 rounded text-xs font-heading transition-colors",
                                                autoBackup.interval_days === d
                                                    ? "bg-[var(--color-accent)] text-black"
                                                    : "bg-black/30 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]"
                                            )}>
                                            {t('settings.backup.auto_interval_days', { count: d })}
                                        </button>
                                    ))}
                                </div>
                            </div>

                            {/* Auto cleanup */}
                            <label className="flex items-center gap-2 text-xs text-[var(--color-text-primary)] cursor-pointer">
                                <input type="checkbox" checked={autoBackup.auto_cleanup}
                                    onChange={e => setAutoBackup(prev => ({ ...prev, auto_cleanup: e.target.checked }))}
                                    className={clsx(toggleClasses, autoBackup.auto_cleanup ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]")}
                                    style={{ appearance: 'none' }}
                                />
                                {t('settings.backup.auto_cleanup')}
                            </label>

                            {autoBackup.auto_cleanup && (
                                <div className="flex items-center gap-2 pl-1">
                                    <Trash2 size={12} className="text-[var(--color-text-muted)]" />
                                    <span className="text-xs text-[var(--color-text-muted)]">{t('settings.backup.auto_keep_prefix')}</span>
                                    <input
                                        type="number"
                                        min={1} max={365}
                                        value={autoBackup.keep_days}
                                        onChange={e => setAutoBackup(prev => ({ ...prev, keep_days: Math.max(1, parseInt(e.target.value) || 1) }))}
                                        className="w-16 px-2 py-1 rounded text-xs bg-black/30 border border-[var(--color-border)] text-[var(--color-text-primary)] text-center"
                                    />
                                    <span className="text-xs text-[var(--color-text-muted)]">{t('settings.backup.auto_keep_suffix')}</span>
                                </div>
                            )}
                        </div>
                    )}

                    {/* Save + Run Now */}
                    <div className="flex gap-2 pt-1">
                        <motion.button whileHover={{ scale: 1.02 }} whileTap={{ scale: 0.98 }}
                            onClick={handleSaveAutoBackup}
                            className={clsx(
                                "flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider",
                                "bg-[var(--color-accent)] text-black hover:bg-white transition-colors"
                            )}>
                            {autoBackupSaved ? <Check size={14} /> : <Clock size={14} />}
                            {autoBackupSaved ? t('common.actions.saved') : t('common.actions.save')}
                        </motion.button>

                        {autoBackup.enabled && autoBackup.backup_dir && (
                            <motion.button whileHover={{ scale: 1.02 }} whileTap={{ scale: 0.98 }}
                                onClick={handleRunNow}
                                disabled={runningNow}
                                className={clsx(
                                    "flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider",
                                    "border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                                    "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                                    "disabled:opacity-50 disabled:cursor-not-allowed"
                                )}>
                                {runningNow ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
                                {t('settings.backup.auto_run_now')}
                            </motion.button>
                        )}
                    </div>

                    {runNowResult && (
                        <div className="flex items-start gap-2 text-xs text-[var(--color-accent)]">
                            <Check size={14} className="mt-0.5 shrink-0" />
                            <span>{t('settings.backup.auto_run_success', { path: runNowResult })}</span>
                        </div>
                    )}
                    {autoBackupError && (
                        <div className="flex items-start gap-2 text-xs text-red-400">
                            <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                            <span>{autoBackupError}</span>
                        </div>
                    )}
                </div>
            </div>

            <div className="border-t border-[var(--color-border)]" />

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
                    <div className="mt-3 flex items-start gap-2 text-xs text-[var(--color-accent)]">
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
                                        style={{ appearance: 'none' }}
                                    />
                                    {t('settings.backup.option_database')}
                                </label>
                            )}
                            {preview.has_configs && (
                                <label className="flex items-center gap-2 text-xs text-[var(--color-text-primary)] cursor-pointer">
                                    <input type="checkbox" checked={importConfigs} onChange={e => setImportConfigs(e.target.checked)}
                                        className={clsx(toggleClasses, importConfigs ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]")}
                                        style={{ appearance: 'none' }}
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
                    <div className="mt-3 flex items-start gap-2 text-xs text-[var(--color-accent)]">
                        <Check size={14} className="mt-0.5 shrink-0" />
                        <span className="whitespace-pre-wrap">{importDone}</span>
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
