// pattern: Functional Core

import { describe, expect, it } from 'vitest';
import {
    backupCredentialWarningKey,
    buildManualExportOptions,
    type BackupResourceMode,
} from './backup-resource-options';

describe('backup resource options', () => {
    it('keeps manual exports data-only by default', () => {
        const mode: BackupResourceMode = 'data-only';

        expect(buildManualExportOptions(mode)).toEqual({ include_character_resources: false });
    });

    it('includes character resources only when explicitly selected', () => {
        expect(buildManualExportOptions('include-resources')).toEqual({
            include_character_resources: true,
        });
    });

    it('keeps the provider credential warning separate from resource selection', () => {
        expect(backupCredentialWarningKey).toBe('settings.backup.provider_credentials_warning');
        expect(buildManualExportOptions('include-resources')).not.toHaveProperty('include_provider_credentials');
    });
});
