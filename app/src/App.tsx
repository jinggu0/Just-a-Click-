import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Source = "microphone" | "system_sound";
type Phase = "idle" | "recording" | "paused" | "stopped" | "failed";

type SourceInfo = { source: Source; device: string | null; format: string | null };

type Status = {
  phase: Phase;
  source: Source | null;
  directory: string | null;
  recorded_seconds: number;
  chunks: number;
  last_chunk: string | null;
  discontinuities: number;
  silence_seconds: number;
  error: string | null;
};

const SOURCE_LABELS: Record<Source, string> = {
  microphone: "마이크",
  system_sound: "시스템 소리",
};

/** Validation screen for the recording checks in the app stack validation. */
function App() {
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [source, setSource] = useState<Source>("microphone");
  const [directory, setDirectory] = useState("");
  const [status, setStatus] = useState<Status | null>(null);
  const [log, setLog] = useState<string[]>([]);

  const note = (message: string) =>
    setLog((entries) => [`${new Date().toLocaleTimeString()} ${message}`, ...entries].slice(0, 20));

  const run = async (command: string, args: Record<string, unknown> = {}) => {
    try {
      const next = await invoke<Status>(command, args);
      setStatus(next);
      note(`${command}: ${next.phase}`);
    } catch (error) {
      note(`${command} 실패: ${String(error)}`);
    }
  };

  useEffect(() => {
    invoke<SourceInfo[]>("list_audio_sources")
      .then((found) => {
        setSources(found);
        note(`장치 조회: ${found.map((item) => `${SOURCE_LABELS[item.source]}=${item.device ?? "없음"}`).join(", ")}`);
      })
      .catch((error) => note(`장치 조회 실패: ${String(error)}`));
  }, []);

  useEffect(() => {
    const timer = setInterval(() => {
      invoke<Status>("recording_status")
        .then(setStatus)
        .catch(() => undefined);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  const recording = status?.phase === "recording" || status?.phase === "paused";

  return (
    <main className="container">
      <h1>녹음 기술 검증</h1>

      <section>
        <h2>입력</h2>
        {sources.map((item) => (
          <label key={item.source} style={{ display: "block" }}>
            <input
              type="radio"
              name="source"
              value={item.source}
              checked={source === item.source}
              disabled={recording}
              onChange={() => setSource(item.source)}
            />
            {SOURCE_LABELS[item.source]} — {item.device ?? "장치 없음"}
            {item.format ? ` (${item.format})` : ""}
          </label>
        ))}
        <label style={{ display: "block", marginTop: "0.5rem" }}>
          저장 폴더
          <input
            value={directory}
            placeholder="C:\녹음\검증"
            disabled={recording}
            onChange={(event) => setDirectory(event.currentTarget.value)}
            style={{ width: "100%" }}
          />
        </label>
      </section>

      <section>
        <h2>제어</h2>
        <button disabled={recording || directory.length === 0} onClick={() => run("start_recording", { source, directory })}>
          시작
        </button>
        <button disabled={status?.phase !== "recording"} onClick={() => run("pause_recording")}>
          일시정지
        </button>
        <button disabled={status?.phase !== "paused"} onClick={() => run("resume_recording")}>
          재개
        </button>
        <button disabled={!recording} onClick={() => run("stop_recording")}>
          정지
        </button>
      </section>

      <section>
        <h2>상태</h2>
        <p>
          단계 {status?.phase ?? "idle"} · 녹음 {status?.recorded_seconds?.toFixed(1) ?? "0.0"}초 · 조각{" "}
          {status?.chunks ?? 0}개 · 불연속 {status?.discontinuities ?? 0}회 · 무음 보정{" "}
          {status?.silence_seconds?.toFixed(1) ?? "0.0"}초
        </p>
        <p>마지막 조각: {status?.last_chunk ?? "없음"}</p>
        {status?.error ? <p style={{ color: "crimson" }}>오류: {status.error}</p> : null}
      </section>

      <section>
        <h2>기록</h2>
        <ul>
          {log.map((entry, index) => (
            <li key={index}>{entry}</li>
          ))}
        </ul>
      </section>
    </main>
  );
}

export default App;
