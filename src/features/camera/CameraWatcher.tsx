import { useEffect, useRef } from "react";
import { setLatestCameraFrame } from "@/lib/camera-frame-cache";

const CAPTURE_INTERVAL_MS = 5000;

interface Props {
    enabled: boolean;
    deviceId?: string;
}

export function CameraWatcher({ enabled, deviceId }: Props) {
    const videoRef = useRef<HTMLVideoElement>(null);
    const streamRef = useRef<MediaStream | null>(null);
    const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        if (!enabled) {
            stopCamera();
            return;
        }
        let cancelled = false;

        async function startCamera() {
            try {
                const constraints = deviceId
                    ? { video: { deviceId: { exact: deviceId } } }
                    : { video: true };
                const stream = await navigator.mediaDevices.getUserMedia(constraints);
                if (cancelled) {
                    stream.getTracks().forEach((t) => t.stop());
                    return;
                }
                streamRef.current = stream;
                if (videoRef.current) {
                    videoRef.current.srcObject = stream;
                    await videoRef.current.play();
                }
                timerRef.current = setInterval(cacheFrame, CAPTURE_INTERVAL_MS);
            } catch (err) {
                console.error("[CameraWatcher] getUserMedia failed:", err);
            }
        }

        startCamera();
        return () => {
            cancelled = true;
            stopCamera();
        };
    }, [enabled, deviceId]);

    function stopCamera() {
        if (timerRef.current) {
            clearInterval(timerRef.current);
            timerRef.current = null;
        }
        streamRef.current?.getTracks().forEach((t) => t.stop());
        streamRef.current = null;
        setLatestCameraFrame(null);
    }

    function cacheFrame() {
        const video = videoRef.current;
        if (!video || video.readyState < 2) return;

        const canvas = document.createElement("canvas");
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        ctx.drawImage(video, 0, 0);
        const dataUrl = canvas.toDataURL("image/jpeg", 0.8);
        setLatestCameraFrame(dataUrl);
    }

    return (
        <video
            ref={videoRef}
            style={{ display: "none" }}
            muted
            playsInline
        />
    );
}
