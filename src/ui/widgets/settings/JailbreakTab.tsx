import React, { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { clsx } from 'clsx';
import { Shield, Download, Upload, Trash2, Save, Check, Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { getJailbreakPrompt, setJailbreakPrompt } from '../../../lib/kokoro-bridge';
import { save, open } from '@tauri-apps/plugin-dialog';
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs';
import { inputClasses, labelClasses } from '../../styles/settings-primitives';

export const JailbreakTab: React.FC = () => {
    const { t } = useTranslation();
    const [prompt, setPrompt] = useState('');
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [saved, setSaved] = useState(false);

    useEffect(() => {
        loadPrompt();
    }, []);

    const loadPrompt = async () => {
        try {
            const loaded = await getJailbreakPrompt();
            setPrompt(loaded);
        } catch (error) {
            console.error('Failed to load jailbreak prompt:', error);
        } finally {
            setLoading(false);
        }
    };

    const handleSave = async () => {
        setSaving(true);
        setSaved(false);
        try {
            await setJailbreakPrompt(prompt);
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (error) {
            console.error('Failed to save jailbreak prompt:', error);
        } finally {
            setSaving(false);
        }
    };

    const handleExport = async () => {
        try {
            const filePath = await save({
                defaultPath: 'jailbreak_prompt.txt',
                filters: [{ name: 'Text', extensions: ['txt'] }],
            });

            if (filePath) {
                await writeTextFile(filePath, prompt);
            }
        } catch (error) {
            console.error('Failed to export prompt:', error);
        }
    };

    const handleImport = async () => {
        try {
            const selected = await open({
                multiple: false,
                filters: [{ name: 'Text', extensions: ['txt'] }],
            });

            if (selected && typeof selected === 'string') {
                const content = await readTextFile(selected);
                setPrompt(content);
            }
        } catch (error) {
            console.error('Failed to import prompt:', error);
        }
    };

    const handleClear = () => {
        if (confirm(t('settings.jailbreak.clearConfirm'))) {
            setPrompt('');
        }
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center py-12">
                <Loader2 size={20} className="animate-spin text-[var(--color-text-muted)]" />
            </div>
        );
    }

    return (
        <div className="space-y-6 pb-6">
            {/* Header */}
            <div className="flex items-start gap-3">
                <Shield size={20} className="text-[var(--color-accent)] mt-1 flex-shrink-0" />
                <div className="flex-1">
                    <h3 className="text-xs font-heading font-semibold text-[var(--color-text-primary)] mb-1 uppercase tracking-wider">
                        {t('settings.jailbreak.title')}
                    </h3>
                    <p className="text-xs text-[var(--color-text-secondary)] leading-relaxed font-sans">
                        {t('settings.jailbreak.description')}
                    </p>
                </div>
            </div>

            {/* Prompt Editor */}
            <div className="space-y-2">
                <label className={labelClasses}>
                    {t('settings.jailbreak.promptLabel')}
                </label>
                <textarea
                    value={prompt}
                    onChange={(e) => setPrompt(e.target.value)}
                    placeholder={t('settings.jailbreak.promptPlaceholder')}
                    className={clsx(
                        inputClasses,
                        'min-h-[300px] !font-mono text-xs resize-y placeholder:!font-mono'
                    )}
                />
                <div className="flex items-center justify-between text-xs">
                    <span className="text-[var(--color-text-muted)] font-sans">
                        {t('settings.jailbreak.charCount', { count: prompt.length })}
                    </span>
                </div>
            </div>

            {/* Action Buttons */}
            <div className="flex items-center gap-2 flex-wrap">
                <motion.button
                    onClick={handleSave}
                    disabled={saving}
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    className={clsx(
                        'flex items-center gap-2 px-4 py-2 rounded-lg font-heading font-semibold text-xs uppercase tracking-wider transition-all',
                        saved
                            ? 'bg-green-500/20 text-green-400 border border-green-500/30'
                            : 'bg-[var(--color-accent)] text-white hover:bg-[var(--color-accent-hover)]',
                        saving && 'opacity-50 cursor-not-allowed'
                    )}
                >
                    {saving ? (
                        <Loader2 size={14} className="animate-spin" />
                    ) : saved ? (
                        <Check size={14} />
                    ) : (
                        <Save size={14} />
                    )}
                    {saved
                        ? t('settings.jailbreak.saved')
                        : saving
                        ? t('settings.jailbreak.saving')
                        : t('settings.jailbreak.save')}
                </motion.button>

                <motion.button
                    onClick={handleExport}
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    className="flex items-center gap-2 px-4 py-2 rounded-lg font-heading font-semibold text-xs uppercase tracking-wider bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] border border-[var(--color-border)] hover:bg-[var(--color-bg-secondary)] transition-all"
                >
                    <Download size={14} />
                    {t('settings.jailbreak.export')}
                </motion.button>

                <motion.button
                    onClick={handleImport}
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    className="flex items-center gap-2 px-4 py-2 rounded-lg font-heading font-semibold text-xs uppercase tracking-wider bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] border border-[var(--color-border)] hover:bg-[var(--color-bg-secondary)] transition-all"
                >
                    <Upload size={14} />
                    {t('settings.jailbreak.import')}
                </motion.button>

                <motion.button
                    onClick={handleClear}
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    className="flex items-center gap-2 px-4 py-2 rounded-lg font-heading font-semibold text-xs uppercase tracking-wider bg-transparent text-red-400 border border-red-500/30 hover:bg-red-500/10 transition-all"
                >
                    <Trash2 size={14} />
                    {t('settings.jailbreak.clear')}
                </motion.button>
            </div>

            {/* Example Section */}
            <div className="mt-6 p-4 rounded-lg bg-[var(--color-bg-tertiary)] border border-[var(--color-border)]">
                <h4 className="text-xs font-heading font-semibold text-[var(--color-text-primary)] mb-2 uppercase tracking-wider">
                    {t('settings.jailbreak.exampleTitle')}
                </h4>
                <pre className="text-xs text-[var(--color-text-secondary)] whitespace-pre-wrap !font-mono leading-relaxed overflow-x-auto">
                    {t('settings.jailbreak.exampleContent')}
                </pre>
            </div>
        </div>
    );
};
