import { commands } from "@/lib/generated/ipc";
import { toIpcJson } from "@/lib/kokoro-bridge";

function invalidArg(command: string, key: string): never {
    throw new Error(`invalid args \`${key}\` for command \`${command}\``);
}

async function invokeModDreamCommand(command: string, request: unknown): Promise<unknown> {
    if (typeof request !== "object" || request === null
        || !("character_id" in request) || typeof request.character_id !== "string") {
        invalidArg(command, "request");
    }
    const character_id = request.character_id;
    if (command === "run_dream_now") return commands.runDreamNow({ character_id });
    if (command === "get_dreaming_summary") return commands.getDreamingSummary({ character_id });

    const limit = "limit" in request ? request.limit : undefined;
    if (limit !== undefined && (typeof limit !== "number" || !Number.isInteger(limit))) invalidArg(command, "request");
    if (command === "list_dream_jobs") return commands.listDreamJobs({ character_id, limit });

    const status = "status" in request ? request.status : null;
    if (status !== null && typeof status !== "string") invalidArg(command, "request");
    return commands.listDreamProposals({ character_id, limit, status });
}

/** Called after the host authorizes the command against the MOD's permissions. */
export async function invokeModCommand(command: string, args: Record<string, unknown>): Promise<unknown> {
    switch (command) {
        case "list_mods": return commands.listMods();
        case "get_mod_theme": return commands.getModTheme();
        case "get_mod_layout": return commands.getModLayout();
        case "get_character_state": return commands.getCharacterState();
        case "get_engine_info": return commands.getEngineInfo();
        case "check_latest_release": return commands.checkLatestRelease();
        case "list_vision_screens": return commands.listVisionScreens();
        case "get_pet_config": return commands.getPetConfig();
        case "show_pet_window": return commands.showPetWindow();
        case "hide_pet_window": return commands.hidePetWindow();
        case "list_actions": return commands.listActions();
        case "get_memory_enabled": return commands.getMemoryEnabled();
        case "get_memory_upgrade_config": return commands.getMemoryUpgradeConfig();
        case "get_context_settings": return commands.getContextSettings();
        case "play_cue":
            if (typeof args.cue !== "string") invalidArg(command, "cue");
            return commands.playCue(args.cue);
        case "get_live2d_model_profile":
            if (typeof args.modelPath !== "string") invalidArg(command, "modelPath");
            return commands.getLive2dModelProfile(args.modelPath);
        case "set_memory_enabled":
            if (typeof args.enabled !== "boolean") invalidArg(command, "enabled");
            return commands.setMemoryEnabled(args.enabled);
        case "get_dreaming_summary":
        case "list_dream_jobs":
        case "list_dream_proposals":
        case "run_dream_now":
            return invokeModDreamCommand(command, args.request);
        case "test_llm_connection":
            return commands.testModLlmConnection(toIpcJson(args.config));
        default:
            throw `Command ${command} not found`;
    }
}
