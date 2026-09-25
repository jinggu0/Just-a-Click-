import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Status = {
  phase: string;
  ready_seconds: number;
  port: number;
  last_answer: string;
  error: string | null;
};

const ROOT = "C:\\temp_git\\Just-a-Click-";

export default function InferenceTab() {
  const [status, setStatus] = useState<Status | null>(null);
  const [chunk, setChunk] = useState("");
  const [log, setLog] = useState<string[]>([]);

  const note = (text: string) =>
    setLog((entries) => [`${new Date().toLocaleTimeString()} ${text}`, ...entries].slice(0, 20));

  const call = async (command: string, args: Record<string, unknown> = {}) => {
    try {
      await invoke(command, args);
      note(`${command} 보냄`);
    } catch (error) {
      note(`${command} 실패: ${error}`);
    }
  };

  useEffect(() => {
    const timer = setInterval(async () => {
      try {
        setStatus(await invoke<Status>("inference_status"));
      } catch (error) {
        note(`상태 조회 실패: ${error}`);
      }
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  return (
    <section>
      <h2>추론 프로세스</h2>
      <p>
        단계 {status?.phase ?? "-"} · 준비 {status?.ready_seconds?.toFixed(1) ?? "0.0"}초 · 포트{" "}
        {status?.port ?? 0}
      </p>
      {status?.error ? <p style={{ color: "#c00" }}>오류: {status.error}</p> : null}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button onClick={() => call("start_inference", { root: ROOT })}>서버 시작</button>
        <button onClick={() => call("run_draft", { prompt: "다음 강의 구간을 요약해 주세요." })}>
          초안 1건
        </button>
        <button onClick={() => call("cancel_draft")}>초안 취소</button>
        <button onClick={() => call("run_transcribe", { root: ROOT, chunk })}>조각 전사</button>
        <button onClick={() => call("cancel_transcribe")}>전사 취소</button>
        <button onClick={() => call("stop_inference")}>서버 정지</button>
      </div>
      <label style={{ display: "block", marginTop: 8 }}>
        전사할 조각 경로
        <input value={chunk} onChange={(event) => setChunk(event.currentTarget.value)} style={{ width: "100%" }} />
      </label>
      <h3>마지막 결과</h3>
      <p style={{ whiteSpace: "pre-wrap" }}>{status?.last_answer || "없음"}</p>
      <h3>기록</h3>
      <ul>
        {log.map((entry, index) => (
          <li key={index}>{entry}</li>
        ))}
      </ul>
    </section>
  );
}
