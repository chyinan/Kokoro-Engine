// pattern: Imperative Shell

import { useEffect, useRef, useCallback } from 'react';
import { modMessageBus } from './ModMessageBus';
import { invoke } from '@tauri-apps/api/core';
import { open as dialogOpen } from '@tauri-apps/plugin-dialog';

/** Message protocol between host ↔ iframe */
interface ModMessage {
    type: 'prop-update' | 'event' | 'action' | 'ready' | 'invoke';
    payload?: unknown;
}

interface IframeSandboxProps {
    src: string;
    id: string;
    permissions?: string[];
    /** Props to forward to the iframe component via postMessage */
    componentProps?: Record<string, unknown>;
    /** Called when the iframe emits an event */
    onModEvent?: (eventName: string, payload: unknown) => void;
}

// ── Blob → Data URL cache ─────────────────────────────
// Converts blob: URLs to base64 data: URLs so cross-origin iframes can render them.
const blobToDataCache = new Map<string, string>();

async function blobUrlToDataUrl(blobUrl: string): Promise<string> {
    if (blobToDataCache.has(blobUrl)) return blobToDataCache.get(blobUrl)!;

    try {
        const response = await fetch(blobUrl);
        const blob = await response.blob();
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onloadend = () => {
                const dataUrl = reader.result as string;
                blobToDataCache.set(blobUrl, dataUrl);
                resolve(dataUrl);
            };
            reader.onerror = reject;
            reader.readAsDataURL(blob);
        });
    } catch (e) {
        console.warn(`[IframeSandbox] Failed to convert blob URL: ${blobUrl}`, e);
        return blobUrl; // fallback to original
    }
}

/**
 * Deep-walk an object and convert all blob: URL strings to data: URLs.
 * Returns a new object (never mutates the original).
 */
async function convertBlobUrls(obj: unknown): Promise<unknown> {
    if (typeof obj === 'string') {
        if (obj.startsWith('blob:')) return blobUrlToDataUrl(obj);
        return obj;
    }

    if (Array.isArray(obj)) {
        return Promise.all(obj.map(item => convertBlobUrls(item)));
    }

    if (obj && typeof obj === 'object') {
        const entries = Object.entries(obj as Record<string, unknown>);
        const converted = await Promise.all(
            entries.map(async ([key, val]) => [key, await convertBlobUrls(val)] as const)
        );
        return Object.fromEntries(converted);
    }

    return obj;
}

// Commands MODs are allowed to invoke via the host proxy.
// Add entries here when new MOD-facing APIs are needed.
const ALLOWED_MOD_COMMANDS = new Set([
    // MOD lifecycle & events
    'list_mods',
    'get_mod_theme',
    'get_mod_layout',
    // Character state (read-only)
    'get_character_state',
    'get_engine_info',
    'check_latest_release',
    'list_vision_screens',
    // Cue / motion control
    'play_cue',
    'play_motion',
    'get_live2d_model_profile',
    // Settings (scoped to MOD namespace)
    'get_mod_setting',
    'set_mod_setting',
    // Chat (send only — no history access)
    'mod_send_message',
    // Conversation history
    // Pet window
    'get_pet_config',
    'show_pet_window',
    'hide_pet_window',
    // Tool settings / MCP action directory
    'list_actions',
    // Memory settings and Dream review
    'get_memory_enabled',
    'set_memory_enabled',
    'get_memory_upgrade_config',
    'get_dreaming_summary',
    'list_dream_jobs',
    'list_dream_proposals',
    'run_dream_now',
    // API / context settings
    'test_llm_connection',
    'get_context_settings',
    // STT local model setup
    // Dialog (file picker)
    'plugin:dialog|open',
]);

/** Commands that are available to a MOD only when its manifest granted the
 * corresponding capability. The grant is checked again at the host boundary
 * for every request; it is not merely UI metadata. */
const MOD_COMMAND_PERMISSIONS: Record<string, string> = {
    'play_cue': 'tts',
    'play_motion': 'tts',
    'get_live2d_model_profile': 'tts',
    'get_tts_config': 'tts',
    'get_memory_enabled': 'memory',
    'set_memory_enabled': 'memory',
    'get_memory_upgrade_config': 'memory',
    'get_dreaming_summary': 'memory',
    'list_dream_jobs': 'memory',
    'list_dream_proposals': 'memory',
    'run_dream_now': 'memory',
    'list_actions': 'mcp',
    'list_mcp_servers': 'mcp',
    'list_vision_screens': 'vision',
    'test_llm_connection': 'system.info',
    'plugin:dialog|open': 'filesystem.read',
};

/** Legacy DOM actions share the same host boundary as invoke(). Keep the
 * allowlist intentionally small; unknown actions fail closed. */
const ALLOWED_MOD_ACTIONS = new Set([
    'close_settings',
    'set_display_mode',
    'set_render_fps',
    'set_background',
    'set_voice_interrupt',
    'send_message',
    'set_model',
    'set_persona',
    'set_language',
    'set_tts_enabled',
    'set_tts_speed',
    'set_tts_pitch',
    'set_tts_playback',
    'test_tts',
    'set_vision_enabled',
    'save_tts_config',
    'save_vision_config',
    'refresh_bot_status',
    'start_bot_platform',
    'stop_bot_platform',
    'list_mcp_servers',
    'add_mcp_server',
    'remove_mcp_server',
    'reconnect_mcp_server',
    'toggle_mcp_server',
    'refresh_mcp_tools',
    'refresh_mods',
]);

const MOD_ACTION_PERMISSIONS: Record<string, string> = {
    set_tts_enabled: 'tts',
    set_tts_speed: 'tts',
    set_tts_pitch: 'tts',
    set_tts_playback: 'tts',
    test_tts: 'tts',
    save_tts_config: 'tts',
    set_vision_enabled: 'vision',
    save_vision_config: 'vision',
    refresh_bot_status: 'bot',
    start_bot_platform: 'bot',
    stop_bot_platform: 'bot',
    list_mcp_servers: 'mcp',
    add_mcp_server: 'mcp',
    remove_mcp_server: 'mcp',
    reconnect_mcp_server: 'mcp',
    toggle_mcp_server: 'mcp',
    refresh_mcp_tools: 'mcp',
};

const MOD_ACTIONS_WITHOUT_PERMISSION = new Set([
    'close_settings',
    'set_display_mode',
    'set_render_fps',
    'set_voice_interrupt',
]);

function hasModPermission(permissions: readonly string[], capability: string | undefined): boolean {
    return capability === undefined || permissions.includes(capability);
}

function isAllowedModCommand(command: string, permissions: readonly string[]): boolean {
    return ALLOWED_MOD_COMMANDS.has(command)
        && hasModPermission(permissions, MOD_COMMAND_PERMISSIONS[command]);
}

function isAllowedModAction(action: string, permissions: readonly string[]): boolean {
    if (!ALLOWED_MOD_ACTIONS.has(action)) return false;
    const capability = MOD_ACTION_PERMISSIONS[action];
    return capability === undefined
        ? MOD_ACTIONS_WITHOUT_PERMISSION.has(action)
        : hasModPermission(permissions, capability);
}

export const IframeSandbox = ({
    src,
    id,
    permissions = [],
    componentProps,
    onModEvent,
}: IframeSandboxProps) => {
    const iframeRef = useRef<HTMLIFrameElement>(null);
    const readyRef = useRef(false);
    // Store the iframe's actual origin once it sends 'ready', so we can use
    // a specific target origin instead of '*' in outgoing postMessages.
    const iframeOriginRef = useRef<string>('*');
    const permissionsRef = useRef<readonly string[]>(permissions);
    permissionsRef.current = permissions;
    const documentGenerationRef = useRef(0);

    // Store volatile callbacks/props in refs to avoid re-triggering the
    // message-listener useEffect (which would unregister & re-register
    // the component on every render).
    const componentPropsRef = useRef(componentProps);
    componentPropsRef.current = componentProps;

    const onModEventRef = useRef(onModEvent);
    onModEventRef.current = onModEvent;

    // Build sandbox attribute — relaxed for better mod DX.
    // allow-same-origin: required for custom protocol loading
    // allow-scripts: required for JS execution
    // allow-forms: allows form submissions inside mod UI
    // allow-modals: allows alert/confirm/prompt dialogs
    // allow-popups: allows window.open (when overlay permission granted)
    // allow-popups + allow-popups-to-escape-sandbox: enables <input type="file"> and Tauri dialogs
    const baseSandbox = 'allow-scripts allow-same-origin allow-forms allow-modals allow-popups allow-popups-to-escape-sandbox';
    const sandboxFlags = baseSandbox;

    // Send props to iframe when they change.
    // postMessage uses the structured clone algorithm which cannot handle
    // functions, symbols, or undefined values. Strip them, then convert
    // blob: URLs to data: URLs so cross-origin iframes can display them.
    const sendToIframe = useCallback(async (msg: ModMessage) => {
        const targetWindow = iframeRef.current?.contentWindow;
        const targetOrigin = iframeOriginRef.current;
        const targetGeneration = documentGenerationRef.current;
        if (!targetWindow || !readyRef.current) return;

        // 1) Strip functions / non-serializable values
        const safe = JSON.parse(JSON.stringify(msg, (_key, value) =>
            typeof value === 'function' ? undefined : value
        ));

        // 2) Convert blob: URLs inside the payload so the iframe can load them
        if (safe.payload) {
            safe.payload = await convertBlobUrls(safe.payload);
        }

        // Use the captured iframe origin instead of '*' to prevent message leakage
        if (documentGenerationRef.current !== targetGeneration
            || iframeRef.current?.contentWindow !== targetWindow
            || !readyRef.current) return;
        targetWindow.postMessage(safe, targetOrigin);
    }, []);

    // Forward prop updates to iframe
    useEffect(() => {
        if (componentProps) {
            sendToIframe({ type: 'prop-update', payload: componentProps });
        }
    }, [componentProps, sendToIframe]);

    // Listen for messages FROM the iframe.
    // Only depends on `id` — props/callbacks are accessed via refs.
    useEffect(() => {
        const handleMessage = (event: MessageEvent<ModMessage>) => {
            if (event.source !== iframeRef.current?.contentWindow) return;

            const msg = event.data;
            if (!msg || typeof msg !== 'object' || !msg.type) return;

            switch (msg.type) {
                case 'ready':
                    readyRef.current = true;
                    // Capture the iframe's origin for targeted postMessage
                    iframeOriginRef.current = event.origin || '*';
                    // Register this iframe with the message bus (pass origin for safe postMessage)
                    if (iframeRef.current?.contentWindow) {
                        modMessageBus.register(id, iframeRef.current.contentWindow, iframeOriginRef.current);
                    }
                    // Send initial props once iframe is ready
                    if (componentPropsRef.current) {
                        sendToIframe({ type: 'prop-update', payload: componentPropsRef.current });
                    }
                    console.log(`[ModFrame ${id}] Component ready (origin: ${event.origin})`);
                    break;

                case 'event':
                    if (onModEventRef.current && msg.payload && typeof msg.payload === 'object') {
                        const { name, ...rest } = msg.payload as { name?: unknown;[key: string]: unknown };
                        if (typeof name !== 'string' || name.trim() === '') break;
                        onModEventRef.current(name, rest);
                    }
                    break;

                case 'action': {
                    const actionPayload = msg.payload as { action?: string; data?: unknown } | undefined;
                    console.log(`[ModFrame ${id}] Action:`, actionPayload);

                    if (!actionPayload?.action || !isAllowedModAction(actionPayload.action, permissionsRef.current)) {
                        console.warn(`[ModFrame ${id}] Blocked ungranted action`, actionPayload?.action);
                        break;
                    }

                    // Route actions to the QuickJS script runtime via Tauri
                    invoke('dispatch_mod_event', {
                        event: `action:${actionPayload.action}`,
                        payload: actionPayload.data ?? null,
                    }).catch((err) => {
                        console.error(`[ModFrame ${id}] Failed to dispatch action:`, err);
                    });

                    // Also emit a DOM CustomEvent so the host React app can react
                    document.dispatchEvent(
                        new CustomEvent('kokoro:mod-action', {
                            detail: { componentId: id, ...actionPayload },
                        })
                    );
                    break;
                }

                // Mod requests a Tauri invoke — proxy through the host window
                case 'invoke': {
                    const invokePayload = msg.payload as {
                        id?: string;
                        command?: string;
                        args?: Record<string, unknown>;
                    } | undefined;

                    if (invokePayload?.command && invokePayload?.id) {
                        const cmd = invokePayload.command;

                        // Security: only allow whitelisted commands
                        if (!isAllowedModCommand(cmd, permissionsRef.current)) {
                            iframeRef.current?.contentWindow?.postMessage({
                                type: 'invoke-result',
                                payload: { id: invokePayload.id, error: `Command '${cmd}' is not permitted for MODs` },
                            }, iframeOriginRef.current);
                            break;
                        }

                        const requestWindow = iframeRef.current?.contentWindow;
                        const requestOrigin = iframeOriginRef.current;
                        const requestGeneration = documentGenerationRef.current;
                        if (!requestWindow || !readyRef.current) break;

                        const respond = (payload: Record<string, unknown>) => {
                            if (documentGenerationRef.current !== requestGeneration
                                || iframeRef.current?.contentWindow !== requestWindow
                                || iframeOriginRef.current !== requestOrigin
                                || !readyRef.current) return;
                            requestWindow.postMessage({ type: 'invoke-result', payload }, requestOrigin);
                        };

                        let promise: Promise<unknown>;

                        if (cmd === 'plugin:dialog|open') {
                            const args = invokePayload.args ?? {};
                            promise = dialogOpen({
                                multiple: args.multiple as boolean ?? false,
                                filters: args.filters as Array<{ name: string; extensions: string[] }> ?? [],
                                title: args.title as string ?? undefined,
                            });
                        } else {
                            promise = invoke(cmd, invokePayload.args ?? {});
                        }

                        promise
                            .then((result) => {
                                respond({ id: invokePayload.id, result });
                            })
                            .catch((err) => {
                                respond({ id: invokePayload.id, error: String(err) });
                            });
                    }
                    break;
                }
            }
        };

        window.addEventListener('message', handleMessage);
        const handleLoad = () => {
            documentGenerationRef.current += 1;
            readyRef.current = false;
            iframeOriginRef.current = '*';
            modMessageBus.unregister(id);
        };
        iframeRef.current?.addEventListener('load', handleLoad);
        return () => {
            window.removeEventListener('message', handleMessage);
            iframeRef.current?.removeEventListener('load', handleLoad);
            documentGenerationRef.current += 1;
            readyRef.current = false;
            iframeOriginRef.current = '*';
            // Unregister from message bus on unmount
            modMessageBus.unregister(id);
        };
    }, [id, src, sendToIframe]);

    return (
        <iframe
            ref={iframeRef}
            src={src}
            title={`Mod Component: ${id}`}
            sandbox={sandboxFlags}
            style={{
                border: 'none',
                width: '100%',
                height: '100%',
                background: 'transparent'
            }}
        />
    );
};
