import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import LaunchersTab from "./components/LaunchersTab";
import CredentialsTab from "./components/CredentialsTab";
import TestLaunchTab from "./components/TestLaunchTab";
import LogPanel from "./components/LogPanel";

type Tab = "launchers" | "credentials" | "test";

const tabs: { id: Tab; label: string; icon: string }[] = [
  { id: "launchers", label: "Launchers & Games", icon: "🎮" },
  { id: "credentials", label: "Credentials", icon: "🔑" },
  { id: "test", label: "Test Launch", icon: "🚀" },
];

export default function App() {
  const [activeTab, setActiveTab] = useState<Tab>("launchers");
  const [logs, setLogs] = useState<{ timestamp: string; level: string; message: string }[]>([]);

  const addLog = useCallback((level: string, message: string) => {
    const timestamp = new Date().toLocaleTimeString("en-US", { hour12: false });
    setLogs((prev) => [...prev.slice(-200), { timestamp, level, message }]);
  }, []);

  const refreshLogs = useCallback(async () => {
    try {
      const dbLogs = await invoke<{ timestamp: string; level: string; message: string }[]>("get_logs");
      if (dbLogs.length > 0) {
        setLogs(dbLogs.slice(-200));
      }
    } catch {
      // DB not ready yet
    }
  }, []);

  useEffect(() => {
    refreshLogs();
  }, []);

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <header className="flex items-center gap-4 px-5 h-12 border-b border-[var(--border)] bg-[var(--bg-secondary)] shrink-0">
        <h1 className="text-sm font-bold tracking-wider text-[var(--accent)] uppercase">
          Credential Lab
        </h1>
        <span className="text-xs text-[var(--text-muted)]">ArcadeOS Dev Tool</span>
      </header>

      {/* Tabs */}
      <div className="flex border-b border-[var(--border)] bg-[var(--bg-secondary)] shrink-0">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`flex items-center gap-2 px-5 py-3 text-sm font-medium transition-colors border-b-2 ${
              activeTab === tab.id
                ? "border-[var(--accent)] text-[var(--accent)]"
                : "border-transparent text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
            }`}
          >
            <span>{tab.icon}</span>
            {tab.label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 min-h-0 overflow-y-auto p-5">
        {activeTab === "launchers" && <LaunchersTab addLog={addLog} />}
        {activeTab === "credentials" && <CredentialsTab addLog={addLog} />}
        {activeTab === "test" && <TestLaunchTab addLog={addLog} />}
      </div>

      {/* Log Panel */}
      <LogPanel logs={logs} />
    </div>
  );
}
