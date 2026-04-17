import { forwardRef, useEffect, useState } from "react";
import type { ForwardRefExoticComponent, RefAttributes } from "react";
import type { Live2DViewerHandle, Live2DViewerProps } from "./Live2DViewer";
import { ensureCubismCoreLoaded } from "./cubism-core-loader";

type LoadedLive2DViewer = ForwardRefExoticComponent<
  Live2DViewerProps & RefAttributes<Live2DViewerHandle>
>;

let live2DViewerModulePromise: Promise<LoadedLive2DViewer> | null = null;

async function loadLive2DViewer(): Promise<LoadedLive2DViewer> {
  if (!live2DViewerModulePromise) {
    live2DViewerModulePromise = ensureCubismCoreLoaded()
      .then(async () => {
        const module = await import("./Live2DViewer");
        return module.default;
      })
      .catch((error) => {
        live2DViewerModulePromise = null;
        throw error;
      });
  }

  return live2DViewerModulePromise;
}

function Live2DLoadError({ message }: { message: string }) {
  return (
    <div
      role="alert"
      className="flex h-full w-full items-center justify-center text-center text-sm text-red-300"
      style={{
        background: "rgba(20, 12, 12, 0.55)",
        backdropFilter: "blur(6px)",
      }}
    >
      {message}
    </div>
  );
}

const Live2DViewerLoader = forwardRef<Live2DViewerHandle, Live2DViewerProps>((props, ref) => {
  const [LoadedComponent, setLoadedComponent] = useState<LoadedLive2DViewer | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let isMounted = true;

    void loadLive2DViewer()
      .then((component) => {
        if (!isMounted) {
          return;
        }

        setLoadedComponent(() => component);
      })
      .catch((error: unknown) => {
        if (!isMounted) {
          return;
        }

        const message = error instanceof Error ? error.message : "unknown Live2D initialization error";
        console.error("[Live2DViewerLoader] Failed to initialize Live2D:", error);
        setLoadError(message);
      });

    return () => {
      isMounted = false;
    };
  }, []);

  if (loadError) {
    return <Live2DLoadError message={`Live2D failed to initialize: ${loadError}`} />;
  }

  if (!LoadedComponent) {
    return null;
  }

  return <LoadedComponent ref={ref} {...props} />;
});

Live2DViewerLoader.displayName = "Live2DViewerLoader";

export default Live2DViewerLoader;
