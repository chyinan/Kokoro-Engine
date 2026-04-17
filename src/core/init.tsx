import { registry } from "../ui/registry/ComponentRegistry";
import ChatPanel from "../ui/widgets/ChatPanel";
import HeaderBar from "../ui/widgets/HeaderBar";
import Live2DViewerLoader from "../features/live2d/Live2DViewerLoader";
import { ModList } from "../ui/mods/ModList";
import SettingsPanel from "../ui/widgets/SettingsPanel";

export function registerCoreComponents() {
    registry.register("ChatPanel", ChatPanel);
    registry.register("SettingsPanel", SettingsPanel);
    registry.register("Live2DStage", Live2DViewerLoader);
    registry.register("HeaderBar", HeaderBar);
    registry.register("ModList", ModList);
}
