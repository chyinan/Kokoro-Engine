import { useEffect, useState } from "react";
import { audioPlayer } from "../../core/services";
import type { AudioAnalysis } from "../../lib/audio-player";

export function AudioDebug() {
    const [amp, setAmp] = useState(0);

    useEffect(() => {
        const handler = (data: AudioAnalysis) => setAmp(data.amplitude);
        audioPlayer.addAmplitudeListener(handler);
        return () => audioPlayer.removeAmplitudeListener(handler);
    }, []);

    const testAudio = async () => {
        console.log("Generating sine wave audio...");

        // Generate a 1-second sine wave at 440Hz as a WAV file
        const sampleRate = 44100;
        const duration = 1; // seconds
        const frequency = 440; // Hz
        const volume = 0.5;
        const numSamples = sampleRate * duration;

        // WAV Header + Data
        const buffer = new ArrayBuffer(44 + numSamples * 2);
        const view = new DataView(buffer);

        // RIFF chunk descriptor
        writeString(view, 0, 'RIFF');
        view.setUint32(4, 36 + numSamples * 2, true);
        writeString(view, 8, 'WAVE');

        // fmt sub-chunk
        writeString(view, 12, 'fmt ');
        view.setUint32(16, 16, true); // Subchunk1Size (16 for PCM)
        view.setUint16(20, 1, true); // AudioFormat (1 for PCM)
        view.setUint16(22, 1, true); // NumChannels (1 for Mono)
        view.setUint32(24, sampleRate, true); // SampleRate
        view.setUint32(28, sampleRate * 2, true); // ByteRate
        view.setUint16(32, 2, true); // BlockAlign
        view.setUint16(34, 16, true); // BitsPerSample

        // data sub-chunk
        writeString(view, 36, 'data');
        view.setUint32(40, numSamples * 2, true);

        // Write samples
        for (let i = 0; i < numSamples; i++) {
            const t = i / sampleRate;
            const sample = Math.sin(t * frequency * 2 * Math.PI) * volume;
            // Convert float [-1, 1] to int16 [-32768, 32767]
            const intSample = sample < 0 ? sample * 32768 : sample * 32767;
            view.setInt16(44 + i * 2, intSample, true);
        }

        const uint8Array = new Uint8Array(buffer);
        await audioPlayer.queueAudio(uint8Array);
    };

    const writeString = (view: DataView, offset: number, string: string) => {
        for (let i = 0; i < string.length; i++) {
            view.setUint8(offset + i, string.charCodeAt(i));
        }
    };

    return (
        <div style={{ position: "fixed", top: 0, left: 0, zIndex: 9999, background: "rgba(0,0,0,0.8)", color: "white", padding: "10px", borderRadius: "0 0 10px 0" }}>
            <h3>Audio Debug</h3>
            <div>Amplitude: {amp.toFixed(3)}</div>
            <div style={{ marginTop: "10px", display: "flex", gap: "5px" }}>
                <button
                    onClick={testAudio}
                    style={{ padding: "5px 10px", background: "#4CAF50", border: "none", color: "white", borderRadius: "3px", cursor: "pointer" }}
                >
                    Test 440Hz
                </button>
                <button
                    onClick={() => audioPlayer.clearQueue()}
                    style={{ padding: "5px 10px", background: "#f44336", border: "none", color: "white", borderRadius: "3px", cursor: "pointer" }}
                >
                    Stop
                </button>
            </div>
        </div>
    );
}
