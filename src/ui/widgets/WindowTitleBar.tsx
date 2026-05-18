import { useCallback, useEffect, useState } from "react";
import { Maximize2, Minimize2, Minus, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";

const isMacLike = () => {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform || "";
  const userAgent = navigator.userAgent || "";
  return /Mac|iPhone|iPad|iPod/i.test(platform) || /Mac OS X/i.test(userAgent);
};

type WindowAction = "minimize" | "maximize" | "close";

function runWindowAction(action: WindowAction) {
  const appWindow = getCurrentWindow();

  if (action === "minimize") {
    return appWindow.minimize();
  }

  if (action === "maximize") {
    return appWindow.toggleMaximize();
  }

  return appWindow.close();
}

type WindowControlsProps = {
  isMaximized: boolean;
  onToggleMaximize: () => void;
  labels: {
    group: string;
    close: string;
    closeWindow: string;
    minimize: string;
    minimizeWindow: string;
    maximize: string;
    maximizeWindow: string;
    restore: string;
    restoreWindow: string;
  };
};

function MacWindowControls({ isMaximized, onToggleMaximize, labels }: WindowControlsProps) {
  const maximizeTitle = isMaximized ? labels.restore : labels.maximize;
  const maximizeAriaLabel = isMaximized ? labels.restoreWindow : labels.maximizeWindow;
  const MaximizeIcon = isMaximized ? Minimize2 : Maximize2;

  return (
    <div className="group flex h-8 items-center gap-2 px-3" aria-label={labels.group}>
      <button
        type="button"
        onClick={() => void runWindowAction("close").catch(console.error)}
        className="flex h-3.5 w-3.5 items-center justify-center rounded-full border border-black/20 bg-[#ff5f57] text-black/55 shadow-[0_0_0_1px_rgba(255,255,255,0.12)_inset]"
        aria-label={labels.closeWindow}
        title={labels.close}
      >
        <X size={8} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-70" />
      </button>
      <button
        type="button"
        onClick={() => void runWindowAction("minimize").catch(console.error)}
        className="flex h-3.5 w-3.5 items-center justify-center rounded-full border border-black/20 bg-[#ffbd2e] text-black/55 shadow-[0_0_0_1px_rgba(255,255,255,0.12)_inset]"
        aria-label={labels.minimizeWindow}
        title={labels.minimize}
      >
        <Minus size={8} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-70" />
      </button>
      <button
        type="button"
        onClick={onToggleMaximize}
        className="flex h-3.5 w-3.5 items-center justify-center rounded-full border border-black/20 bg-[#28c840] text-black/55 shadow-[0_0_0_1px_rgba(255,255,255,0.12)_inset]"
        aria-label={maximizeAriaLabel}
        title={maximizeTitle}
      >
        <MaximizeIcon size={7} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-70" />
      </button>
    </div>
  );
}

function StandardWindowControls({ isMaximized, onToggleMaximize, labels }: WindowControlsProps) {
  const maximizeTitle = isMaximized ? labels.restore : labels.maximize;
  const maximizeAriaLabel = isMaximized ? labels.restoreWindow : labels.maximizeWindow;
  const MaximizeIcon = isMaximized ? Minimize2 : Maximize2;

  return (
    <div className="group flex h-9 items-center gap-2 px-3" aria-label={labels.group}>
      <button
        type="button"
        onClick={() => void runWindowAction("minimize").catch(console.error)}
        className="flex h-4 w-4 items-center justify-center rounded-full border border-black/25 bg-[#ffbd2e] text-black/60 shadow-[0_0_0_1px_rgba(255,255,255,0.16)_inset,0_2px_8px_rgba(0,0,0,0.24)] transition-transform hover:scale-110"
        aria-label={labels.minimizeWindow}
        title={labels.minimize}
      >
        <Minus size={9} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-75" />
      </button>
      <button
        type="button"
        onClick={onToggleMaximize}
        className="flex h-4 w-4 items-center justify-center rounded-full border border-black/25 bg-[#28c840] text-black/60 shadow-[0_0_0_1px_rgba(255,255,255,0.16)_inset,0_2px_8px_rgba(0,0,0,0.24)] transition-transform hover:scale-110"
        aria-label={maximizeAriaLabel}
        title={maximizeTitle}
      >
        <MaximizeIcon size={8} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-75" />
      </button>
      <button
        type="button"
        onClick={() => void runWindowAction("close").catch(console.error)}
        className="flex h-4 w-4 items-center justify-center rounded-full border border-black/25 bg-[#ff5f57] text-black/60 shadow-[0_0_0_1px_rgba(255,255,255,0.16)_inset,0_2px_8px_rgba(0,0,0,0.24)] transition-transform hover:scale-110"
        aria-label={labels.closeWindow}
        title={labels.close}
      >
        <X size={9} strokeWidth={2.5} className="opacity-0 transition-opacity group-hover:opacity-75" />
      </button>
    </div>
  );
}

export default function WindowTitleBar() {
  const { t } = useTranslation();
  const isMac = isMacLike();
  const [isMaximized, setIsMaximized] = useState(false);
  const labels = {
    group: t("common.window_controls.group"),
    close: t("common.window_controls.close"),
    closeWindow: t("common.window_controls.close_window"),
    minimize: t("common.window_controls.minimize"),
    minimizeWindow: t("common.window_controls.minimize_window"),
    maximize: t("common.window_controls.maximize"),
    maximizeWindow: t("common.window_controls.maximize_window"),
    restore: t("common.window_controls.restore"),
    restoreWindow: t("common.window_controls.restore_window"),
  };

  const syncMaximized = useCallback(async () => {
    try {
      const maximized = await getCurrentWindow().isMaximized();
      setIsMaximized(maximized);
    } catch (error) {
      console.error(error);
    }
  }, []);

  const toggleMaximize = useCallback(async () => {
    try {
      const appWindow = getCurrentWindow();
      await appWindow.toggleMaximize();
      const maximized = await appWindow.isMaximized();
      setIsMaximized(maximized);
      window.setTimeout(() => void syncMaximized(), 100);
    } catch (error) {
      console.error(error);
    }
  }, [syncMaximized]);

  useEffect(() => {
    let disposed = false;
    const appWindow = getCurrentWindow();
    const sync = async () => {
      try {
        const maximized = await appWindow.isMaximized();
        if (!disposed) setIsMaximized(maximized);
      } catch (error) {
        console.error(error);
      }
    };
    const handleResize = () => void sync();
    const unlistenResized = appWindow.onResized(handleResize);

    void sync();
    window.addEventListener("resize", handleResize);

    return () => {
      disposed = true;
      window.removeEventListener("resize", handleResize);
      void unlistenResized.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);

  return (
    <>
      <div
        data-tauri-drag-region
        onDoubleClick={toggleMaximize}
        className="fixed left-0 right-0 top-0 z-[70] h-10 bg-transparent"
      />
      <div
        className={[
          "fixed top-1 z-[120] bg-transparent",
          isMac ? "left-0" : "right-0",
        ].join(" ")}
      >
        {isMac ? (
          <MacWindowControls
            isMaximized={isMaximized}
            onToggleMaximize={toggleMaximize}
            labels={labels}
          />
        ) : (
          <StandardWindowControls
            isMaximized={isMaximized}
            onToggleMaximize={toggleMaximize}
            labels={labels}
          />
        )}
      </div>
    </>
  );
}
