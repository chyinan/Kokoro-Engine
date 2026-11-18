import { commands } from "@/lib/generated/ipc";
import type { ModManifest } from "../types/mod";

export class ModService {
    async listMods(): Promise<ModManifest[]> {
        return commands.listMods();
    }

    async loadMod(modId: string): Promise<void> {
        await commands.loadMod(modId);
    }
}

export const modService = new ModService();
