// pattern: Functional Core

export type BackupResourceMode = 'data-only' | 'include-resources';

export type ManualExportOptions = {
    readonly include_character_resources: boolean;
};

export const backupCredentialWarningKey = 'settings.backup.provider_credentials_warning';

export function buildManualExportOptions(mode: BackupResourceMode): ManualExportOptions {
    return { include_character_resources: mode === 'include-resources' };
}
