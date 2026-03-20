import { useRef, useEffect } from "react";

interface LogEntry {
  timestamp: string;
  level: string;
  message: string;
}

export default function LogPanel({ logs }: { logs: LogEntry[] }) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs.length]);

  const levelColor = (level: string) => {
    switch (level) {
      case "error": return "text-[var(--danger)]";
      case "warn": return "text-[var(--warning)]";
      default: return "text-[var(--text-muted)]";
    }
  };

  return (
    <div className="h-40 border-t border-[var(--border)] bg-[#08080a] shrink-0 flex flex-col">
      <div className="flex items-center justify-between px-4 py-1.5 border-b border-[var(--border)]">
        <span className="text-xs font-semibold text-[var(--text-muted)] uppercase tracking-wider">
          Operation Log
        </span>
        <span className="text-xs text-[var(--text-muted)]">{logs.length} entries</span>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-2 font-mono text-xs space-y-0.5">
        {logs.length === 0 && (
          <p className="text-[var(--text-muted)] italic">No operations yet...</p>
        )}
        {logs.map((log, i) => (
          <div key={i} className="flex gap-2">
            <span className="text-[var(--text-muted)] shrink-0">[{log.timestamp}]</span>
            <span className={levelColor(log.level)}>{log.message}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
