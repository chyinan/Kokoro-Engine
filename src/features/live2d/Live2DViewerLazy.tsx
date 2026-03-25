import { lazy, Suspense } from "react";
import type { Live2DViewerProps, Live2DViewerHandle } from "./Live2DViewer";

const Live2DViewerComponent = lazy(() => import("./Live2DViewer"));

export const Live2DViewerLazy = (props: Live2DViewerProps & { ref?: React.Ref<Live2DViewerHandle> }) => (
  <Suspense fallback={<div className="w-full h-full bg-black" />}>
    <Live2DViewerComponent {...props} />
  </Suspense>
);
