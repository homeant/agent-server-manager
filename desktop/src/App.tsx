import { useEffect, useState } from "react";
import { api } from "./lib/bridge";
import { getStoredLocale, resolveLocale, saveLocale, translate, type AppLocale, type LocalePreference, type TranslationKey } from "./lib/i18n";
import type { LogLine, ServiceInfo, ServiceSpec, ServiceStatus } from "./lib/types";

type IconName =
  | "grid"
  | "plus"
  | "refresh"
  | "play"
  | "stop"
  | "restart"
  | "trash"
  | "terminal"
  | "sliders"
  | "chevron"
  | "close"
  | "search"
  | "external"
  | "info"
  | "check";

function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.8,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };

  switch (name) {
    case "grid":
      return <svg {...common}><rect x="4" y="4" width="6" height="6" rx="1" /><rect x="14" y="4" width="6" height="6" rx="1" /><rect x="4" y="14" width="6" height="6" rx="1" /><rect x="14" y="14" width="6" height="6" rx="1" /></svg>;
    case "plus":
      return <svg {...common}><path d="M12 5v14M5 12h14" /></svg>;
    case "refresh":
      return <svg {...common}><path d="M20 11a8 8 0 0 0-14.7-3.9L4 9" /><path d="M4 4v5h5" /><path d="M4 13a8 8 0 0 0 14.7 3.9L20 15" /><path d="M20 20v-5h-5" /></svg>;
    case "play":
      return <svg {...common}><path d="m8 5 11 7-11 7V5Z" /></svg>;
    case "stop":
      return <svg {...common}><rect x="6" y="6" width="12" height="12" rx="1.5" /></svg>;
    case "restart":
      return <svg {...common}><path d="M20 11a8 8 0 0 0-14.7-3.9L4 9" /><path d="M4 4v5h5" /><path d="M20 4v5h-5" /><path d="M20 13a8 8 0 0 1-14.7 3.9L4 15" /></svg>;
    case "trash":
      return <svg {...common}><path d="M4 7h16M10 11v5M14 11v5M6 7l1 13h10l1-13M9 7V4h6v3" /></svg>;
    case "terminal":
      return <svg {...common}><rect x="3.5" y="4" width="17" height="16" rx="2" /><path d="m7 9 3 3-3 3M13 15h4" /></svg>;
    case "sliders":
      return <svg {...common}><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></svg>;
    case "chevron":
      return <svg {...common}><path d="m9 5 7 7-7 7" /></svg>;
    case "close":
      return <svg {...common}><path d="m6 6 12 12M18 6 6 18" /></svg>;
    case "search":
      return <svg {...common}><circle cx="10.8" cy="10.8" r="6.3" /><path d="m16 16 4 4" /></svg>;
    case "external":
      return <svg {...common}><path d="M14 5h5v5M19 5l-8 8" /><path d="M19 14v4a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h4" /></svg>;
    case "info":
      return <svg {...common}><circle cx="12" cy="12" r="8.5" /><path d="M12 10.5v5M12 7.5h.01" /></svg>;
    case "check":
      return <svg {...common}><path d="m5 12 4 4L19 6" /></svg>;
  }
}

function statusText(status: ServiceStatus, locale: AppLocale) {
  const key: TranslationKey = status === "running"
    ? "running"
    : status === "stopped"
      ? "stopped"
      : status === "starting"
        ? "statusStarting"
        : status === "stopping"
          ? "statusStopping"
          : status === "exited"
            ? "statusExited"
            : "statusErrored";
  return translate(locale, key);
}

function statusClass(status: ServiceStatus) {
  if (status === "running") return "running";
  if (status === "stopped") return "stopped";
  return "attention";
}

function StatusBadge({ status, compact = false, locale }: { status: ServiceStatus; compact?: boolean; locale: AppLocale }) {
  return (
    <span className={`status-badge ${statusClass(status)} ${compact ? "compact" : ""}`}>
      <span className="status-dot" />
      {statusText(status, locale)}
    </span>
  );
}

function formatBytes(value?: number) {
  if (!value) return "—";
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / 1024 / 1024).toFixed(0)} MB`;
}

function formatUptime(startedAt: number | undefined, locale: AppLocale) {
  if (!startedAt) return "—";
  const seconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  if (locale === "zh-CN") {
    if (seconds < 60) return `${seconds}秒`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}分钟`;
    const hours = Math.floor(minutes / 60);
    const remainder = minutes % 60;
    return `${hours}小时${remainder ? ` ${remainder}分钟` : ""}`;
  }
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return `${hours}h ${remainder}m`;
}

function formatTime(ts: number, locale: AppLocale) {
  return new Intl.DateTimeFormat(locale === "zh-CN" ? "zh-CN" : "en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(ts);
}

function Metric({ label, value, caption, tone }: { label: string; value: number; caption: string; tone: string }) {
  return (
    <div className="metric">
      <div className="metric-label">{label}</div>
      <div className={`metric-value ${tone}`}>{value}</div>
      <div className="metric-caption">{caption}</div>
    </div>
  );
}

function EmptyInspector({ onAdd }: { onAdd: () => void }) {
  return (
    <aside className="inspector empty-inspector">
      <div className="empty-mark"><Icon name="terminal" size={24} /></div>
      <h2>Choose a service</h2>
      <p>Select a service to inspect its process, command and live output.</p>
      <button className="button secondary" onClick={onAdd}><Icon name="plus" size={15} /> Add service</button>
    </aside>
  );
}

function App() {
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [view, setView] = useState<"overview" | "service">("overview");
  const [localePreference, setLocalePreference] = useState<LocalePreference>(() => getStoredLocale());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [selectedName, setSelectedName] = useState<string>();
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [loading, setLoading] = useState(true);
  const [daemonConnected, setDaemonConnected] = useState(false);
  const [busy, setBusy] = useState<string>();
  const [toast, setToast] = useState<string>();
  const [error, setError] = useState<string>();
  const [addOpen, setAddOpen] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<ServiceInfo>();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [copyMenu, setCopyMenu] = useState<{ x: number; y: number; text: string }>();
  const [form, setForm] = useState({ name: "", command: "", cwd: "", port: "", autorestart: false });

  const locale = resolveLocale(localePreference);
  const t = (key: TranslationKey, values?: Record<string, string | number>) => translate(locale, key, values);
  const selected = view === "service" ? services.find((service) => service.name === selectedName) : undefined;
  const running = services.filter((service) => service.status === "running").length;
  const stopped = services.filter((service) => service.status === "stopped").length;
  const attention = services.length - running - stopped;

  async function refreshServices(quiet = false) {
    if (!quiet) setLoading(true);
    try {
      const [nextServices, connected] = await Promise.all([api.getServices(), api.daemonStatus()]);
      setServices(nextServices);
      setDaemonConnected(connected);
      setSelectedName((current) => current && nextServices.some((service) => service.name === current) ? current : undefined);
      setError(undefined);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setDaemonConnected(false);
    } finally {
      setLoading(false);
    }
  }

  async function refreshLogs(name: string) {
    try {
      setLogs(await api.getLogs(name));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  useEffect(() => {
    void refreshServices();
    const timer = window.setInterval(() => void refreshServices(true), 2500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!selectedName) {
      setLogs([]);
      return;
    }
    void refreshLogs(selectedName);
    const timer = window.setInterval(() => void refreshLogs(selectedName), 3500);
    return () => window.clearInterval(timer);
  }, [selectedName]);

  useEffect(() => {
    setDetailsOpen(false);
  }, [selectedName]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(undefined), 3200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    void api.setLocale(locale).catch((reason) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [locale]);

  async function action(label: string, operation: () => Promise<unknown>) {
    setBusy(label);
    setError(undefined);
    try {
      await operation();
      await refreshServices(true);
      setToast(label);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(undefined);
    }
  }

  function openAdd() {
    setSettingsOpen(false);
    setForm({ name: "", command: "", cwd: "", port: "", autorestart: false });
    setAddOpen(true);
  }

  async function submitAdd() {
    if (!form.name.trim() || !form.command.trim()) {
      setError(t("errorNameCommandRequired"));
      return;
    }
    const spec: ServiceSpec = {
      name: form.name.trim(),
      command: form.command.trim(),
      cwd: form.cwd.trim() || ".",
      port: form.port.trim() ? Number(form.port) : undefined,
      autorestart: form.autorestart,
    };
    await action(t("toastRegistered"), async () => {
      await api.registerService(spec);
      setView("service");
      setSelectedName(spec.name);
      setAddOpen(false);
    });
  }

  async function confirmRemove() {
    if (!removeTarget) return;
    const name = removeTarget.name;
    await action(t("toastRemoved", { name }), async () => {
      await api.removeService(name);
      setRemoveTarget(undefined);
      if (selectedName === name) {
        setView("overview");
        setSelectedName(undefined);
      }
    });
  }

  function showOverview() {
    setSettingsOpen(false);
    setView("overview");
    setSelectedName(undefined);
  }

  function inspectService(name: string) {
    setSettingsOpen(false);
    setSelectedName(name);
    setView("service");
  }

  function changeLocale(preference: LocalePreference) {
    setLocalePreference(preference);
    saveLocale(preference);
    setSettingsOpen(false);
  }

  async function copySelectedText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      textarea.remove();
    }
    setCopyMenu(undefined);
  }

  return (
    <div className="app-shell" onMouseDown={() => setCopyMenu(undefined)} onContextMenu={(event) => {
      const selection = window.getSelection()?.toString() ?? "";
      const canCopySelection = Boolean(selection.trim());
      if (canCopySelection) {
        event.preventDefault();
        const menuWidth = 104;
        const menuHeight = 38;
        setCopyMenu({
          x: Math.min(event.clientX, Math.max(8, window.innerWidth - menuWidth - 8)),
          y: Math.min(event.clientY, Math.max(8, window.innerHeight - menuHeight - 8)),
          text: selection,
        });
        return;
      }
      setCopyMenu(undefined);
      event.preventDefault();
    }}>
      <div
        className={`window-drag-region ${selected ? "has-title" : ""}`}
        onMouseDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          if (event.detail === 2) {
            void api.toggleMaximize();
          } else {
            void api.startDragging();
          }
        }}
      >
        {selected && <div className="window-titlebar-copy">
          <span className="window-titlebar-title">{selected.name}</span>
          <span className={`window-titlebar-status ${statusClass(selected.status)}`}>
            <span className="status-dot" />
            {statusText(selected.status, locale)}
          </span>
        </div>}
      </div>
      {copyMenu && <div className="copy-context-menu" role="menu" style={{ left: copyMenu.x, top: copyMenu.y }} onMouseDown={(event) => event.stopPropagation()}><button type="button" role="menuitem" onClick={() => void copySelectedText(copyMenu.text)}>{t("copy")}</button></div>}
      <aside className="sidebar">
        <div className="sidebar-section-label">{t("workspace")}</div>
        <nav className="primary-nav" aria-label={t("workspace")}>
          <button className={`nav-item ${view === "overview" ? "active" : ""}`} onClick={showOverview}><Icon name="grid" size={17} /><span>{t("overview")}</span><span className="nav-kbd">⌘ 1</span></button>
        </nav>

        <div className="sidebar-section-label"><span>{t("services")}</span><span className="sidebar-section-actions"><button className="icon-button sidebar-refresh" onClick={() => void refreshServices()} disabled={loading} aria-label={t("refreshServices")} title={t("refreshServices")}><Icon name="refresh" size={13} /></button><button className="icon-button sidebar-add" onClick={openAdd} aria-label={t("addService")} title={t("addService")}><Icon name="plus" size={14} /></button></span></div>
        <div className="sidebar-services">
          {services.map((service) => (
            <button key={service.name} className={`sidebar-service ${view === "service" && selectedName === service.name ? "selected" : ""}`} onClick={() => inspectService(service.name)} aria-label={`${service.name} — ${statusText(service.status, locale)}`} title={statusText(service.status, locale)}>
              <span className={`sidebar-dot ${statusClass(service.status)}`} />
              <span className="sidebar-service-copy"><strong>{service.name}</strong></span>
              <Icon name="chevron" size={14} />
            </button>
          ))}
          {!services.length && !loading && <div className="sidebar-empty">{t("nothingRegistered")}</div>}
        </div>

        <div className="sidebar-footer">
          <div className="sidebar-settings">
            <button className={`sidebar-settings-button ${settingsOpen ? "open" : ""}`} onClick={() => setSettingsOpen((open) => !open)} aria-haspopup="menu" aria-expanded={settingsOpen}>
              <Icon name="sliders" size={15} /><span>{t("settings")}</span><span className="sidebar-settings-current">{localePreference === "auto" ? t("auto") : localePreference === "zh-CN" ? t("chinese") : t("english")}</span>
            </button>
            {settingsOpen && <div className="settings-menu" role="menu">
              <div className="settings-menu-title">{t("language")}</div>
              {(["auto", "zh-CN", "en"] as LocalePreference[]).map((option) => {
                const label = option === "auto" ? t("auto") : option === "zh-CN" ? t("chinese") : t("english");
                return <button key={option} className={`settings-option ${localePreference === option ? "selected" : ""}`} role="menuitemradio" aria-checked={localePreference === option} onClick={() => changeLocale(option)}><span>{label}</span>{localePreference === option && <Icon name="check" size={14} />}</button>;
              })}
              <div className="settings-menu-divider" />
              <div className="settings-menu-title">{t("daemonStatus")}</div>
              <div className="settings-daemon-row"><span className={`settings-daemon-dot ${daemonConnected ? "online" : "offline"}`} /><span><strong>{t("localDaemon")}</strong><small>{daemonConnected ? t("connected") : t("notConnected")}</small></span></div>
            </div>}
          </div>
        </div>
      </aside>

      <main className="main-content">
        <div className={`content-scroll ${selected ? "detail-mode" : "overview-mode"}`}>
          {!selected && <>
            <section className="page-heading">
              <div>
                <div className="eyebrow">{t("overviewEyebrow")}</div>
                <h1>{t("overviewTitle")}<span className="heading-period">.</span></h1>
                <p>{t("overviewSubtitle")}</p>
              </div>
              <div className="heading-actions">
                <button className="button dark" disabled={!!busy} onClick={() => void action(t("toastAllStarted"), () => api.startAll())}><Icon name="play" size={14} /> {t("startAll")}</button>
                <button className="button ghost" disabled={!!busy} onClick={() => void action(t("toastAllStopped"), () => api.stopAll())}><Icon name="stop" size={14} /> {t("stopAll")}</button>
              </div>
            </section>

            <section className="metrics" aria-label={t("overview") }>
              <Metric label={t("running").toUpperCase()} value={running} caption={t("onlineNow")} tone="green" />
              <Metric label={t("stopped").toUpperCase()} value={stopped} caption={t("readyToStart")} tone="muted" />
              <Metric label={t("attention").toUpperCase()} value={attention} caption={attention ? t("needsReview") : t("everythingLooksGood")} tone={attention ? "red" : "muted"} />
              <div className="metrics-note"><span className="metrics-note-mark"><Icon name="check" size={14} /></span><span><strong>{services.length ? t("daemonHealthy") : t("readyFirstService")}</strong><small>{services.length ? t("noDuplicateProcesses") : t("registerCommand")}</small></span></div>
            </section>
          </>}

          <section className="workspace" id="services">
            {selected ? <aside className="inspector">
              <div className="inspector-actions">
                {selected.status === "running" ? <button className="button inspector-button danger" disabled={!!busy} onClick={() => void action(t("toastServiceStopped", { name: selected.name }), () => api.stopService(selected.name))}><Icon name="stop" size={14} /> {t("stop")}</button> : <button className="button inspector-button lime" disabled={!!busy} onClick={() => void action(t("toastServiceStarted", { name: selected.name }), () => api.startService(selected.name))}><Icon name="play" size={14} /> {t("start")}</button>}
                <button className="button inspector-button" disabled={!!busy} onClick={() => void action(t("toastServiceRestarted", { name: selected.name }), () => api.restartService(selected.name))}><Icon name="restart" size={14} /> {t("restart")}</button>
                <button className={`button details-toggle ${detailsOpen ? "open" : ""}`} onClick={() => setDetailsOpen((open) => !open)} aria-haspopup="dialog" aria-expanded={detailsOpen}><Icon name="info" size={14} /><span>{t("details")}</span></button>
                <button className="button icon-only danger" disabled={!!busy} onClick={() => setRemoveTarget(selected)} aria-label={t("removeService")} title={t("removeService")}><Icon name="trash" size={15} /></button>
              </div>
              {detailsOpen && <div className="details-popover-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setDetailsOpen(false)}>
                <div className="details-popover" role="dialog" aria-label={t("details")} onMouseDown={(event) => event.stopPropagation()}>
                  <div className="details-popover-heading"><div><div className="section-label">{t("details")}</div><strong>{selected.name}</strong></div><button className="icon-button" onClick={() => setDetailsOpen(false)} aria-label={t("close")} title={t("close")}><Icon name="close" size={15} /></button></div>
                  <div className="details-popover-command"><div className="section-label">{t("command")}</div><div className="command-box"><Icon name="terminal" size={15} /><code>{selected.command || t("noCommand")}</code></div></div>
                  <div className="details-popover-list detail-list"><div><span>{t("workingDirectory")}</span><strong title={selected.cwd}>{selected.cwd || "—"}</strong></div><div><span>{t("restartsSession")}</span><strong>{selected.restarts}</strong></div><div><span>{t("autoRestart")}</span><strong>{selected.autorestart ? t("enabled") : t("disabled")}</strong></div></div>
                </div>
              </div>}
              <div className="detail-stats">
                <div><small>{t("pid")}</small><strong>{selected.pid ?? "—"}</strong></div>
                <div><small>{t("port")}</small><strong>{selected.port ?? "—"}</strong></div>
                <div><small>{t("memory")}</small><strong>{formatBytes(selected.memoryBytes)}</strong></div>
                <div><small>{t("cpuUsage")}</small><strong>{selected.cpuPercent !== undefined ? `${selected.cpuPercent.toFixed(1)}%` : "—"}</strong></div>
                <div><small>{t("uptime")}</small><strong>{formatUptime(selected.startedAt, locale)}</strong></div>
              </div>
              <div className="logs-section primary-logs"><div className="logs-heading"><div><div className="section-label">{t("liveOutput")}</div><span>{t("lines", { count: logs.length })} <span className="logs-live"><i /> {t("live")}</span></span></div><button className="text-button" onClick={() => void refreshLogs(selected.name)} aria-label={t("refresh")}><Icon name="refresh" size={13} /> {t("refresh")}</button></div><div className="log-view">{logs.length ? logs.map((log, index) => <div className="log-line" key={`${log.ts}-${index}`}><span className="log-time">{formatTime(log.ts, locale)}</span><span className={`log-stream ${log.stream}`}>{log.stream === "stderr" ? "ERR" : log.stream === "system" ? "SYS" : "OUT"}</span><span className="log-text">{log.line}</span></div>) : <div className="log-empty">{t("noOutput")}</div>}</div></div>
            </aside> : <div className="pulse-panel">
              <div className="panel-heading">
                <div><h2>{t("stackPulse")}</h2><p>{t("stackPulseSubtitle")}</p></div>
                <StatusBadge status={attention ? "errored" : "running"} compact locale={locale} />
              </div>
              <div className="pulse-body">
                <div className="pulse-ring" style={{ background: `conic-gradient(var(--green) ${services.length ? Math.round((running / services.length) * 100) : 0}%, var(--line) 0)` }}><div><strong>{running}</strong><span>{t("running")}</span></div></div>
                <div className="pulse-copy">
                  <div className="pulse-status-row"><span className="pulse-status-label"><i className="green" /> {t("running")}</span><strong>{running}</strong><small>{t("onlineNow")}</small></div>
                  <div className="pulse-status-row"><span className="pulse-status-label"><i className="gray" /> {t("stopped")}</span><strong>{stopped}</strong><small>{t("readyToStart")}</small></div>
                  <div className="pulse-status-row"><span className="pulse-status-label"><i className="red" /> {t("attention")}</span><strong>{attention}</strong><small>{attention ? t("needsReview") : t("allClear")}</small></div>
                  <p className="pulse-hint">{t("chooseServiceHint")}</p>
                </div>
              </div>
              <div className="pulse-footer"><span><Icon name="check" size={14} /> {daemonConnected ? t("daemonConnected") : t("waitingDaemon")}</span><span>{t("servicesRegistered", { count: services.length })}</span></div>
            </div>}
          </section>
        </div>
      </main>

      {error && <div className="error-banner"><span>{error}</span><button onClick={() => setError(undefined)}><Icon name="close" size={14} /></button></div>}
      {toast && <div className="toast"><span className="toast-check"><Icon name="check" size={13} /></span>{toast}</div>}

      {addOpen && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setAddOpen(false)}><div className="modal-sheet"><div className="modal-heading"><div><div className="eyebrow">{t("newRegistration")}</div><h2>{t("addServiceTitle")}</h2><p>{t("addServiceDescription")}</p></div><button className="icon-button" onClick={() => setAddOpen(false)} aria-label={t("close")}><Icon name="close" size={17} /></button></div><div className="form-grid"><label><span>{t("serviceName")}</span><input autoFocus value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="e.g. api" /></label><label><span>{t("portLabel")} <em>{t("optional")}</em></span><input type="number" value={form.port} onChange={(event) => setForm({ ...form, port: event.target.value })} placeholder="3000" /></label><label className="full"><span>{t("commandLabel")}</span><input value={form.command} onChange={(event) => setForm({ ...form, command: event.target.value })} placeholder="npm run dev" /></label><label className="full"><span>{t("workingDirectoryLabel")} <em>{t("optional")}</em></span><input value={form.cwd} onChange={(event) => setForm({ ...form, cwd: event.target.value })} placeholder="/Users/you/project" /></label></div><label className="toggle-row"><input type="checkbox" checked={form.autorestart} onChange={(event) => setForm({ ...form, autorestart: event.target.checked })} /><span className="toggle"><i /></span><span><strong>{t("restartAutomatically")}</strong><small>{t("restartDescription")}</small></span></label><div className="modal-actions"><button className="button ghost" onClick={() => setAddOpen(false)}>{t("cancel")}</button><button className="button primary" disabled={!!busy} onClick={() => void submitAdd()}><Icon name="play" size={14} /> {t("registerStart")}</button></div></div></div>}

      {removeTarget && <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && setRemoveTarget(undefined)}><div className="confirm-dialog"><div className="confirm-icon"><Icon name="trash" size={20} /></div><div><div className="eyebrow">{t("removeServiceTitle")}</div><h2>{t("removeServiceQuestion", { name: removeTarget.name })}</h2><p>{t("removeServiceDescription")}</p></div><div className="modal-actions"><button className="button ghost" onClick={() => setRemoveTarget(undefined)}>{t("cancel")}</button><button className="button red" disabled={!!busy} onClick={() => void confirmRemove()}>{t("removeService")}</button></div></div></div>}
    </div>
  );
}

export default App;
