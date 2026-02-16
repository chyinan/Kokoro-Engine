import { invoke } from "@tauri-apps/api/core";
import { ModManifest } from "../types/mod";

export class ModService {
    async listMods(): Promise<ModManifest[]> {
        return invoke<ModManifest[]>("list_mods");
    }

    async loadMod(modId: string): Promise<void> {
        return invoke("load_mod", { modId });
    }
}

export const modService = new ModService();
