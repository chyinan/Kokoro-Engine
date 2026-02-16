import { registry } from "../ui/registry/ComponentRegistry";
import ChatPanel from "../ui/widgets/ChatPanel";
import HeaderBar from "../ui/widgets/HeaderBar";
import FooterBar from "../ui/widgets/FooterBar";
import Live2DViewer from "../features/live2d/Live2DViewer";
import { ModList } from "../ui/mods/ModList";

export function registerCoreComponents() {
    registry.register("ChatPanel", ChatPanel);
    registry.register("Live2DStage", Live2DViewer);
    registry.register("HeaderBar", HeaderBar);
    registry.register("FooterBar", FooterBar);
    registry.register("ModList", ModList);
}
