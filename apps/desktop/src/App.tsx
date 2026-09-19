import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { meetingApi, type Meeting } from "./meetingApi";
import "./App.css";

type View = "home" | "meetings" | "record" | "settings";
type IconName = "home" | "meetings" | "record" | "settings" | "search" | "plus" | "clock" | "sparkles" | "shield" | "headphones" | "chevron";
const paths: Record<IconName, ReactNode> = {
  home: <><path d="m3 11 9-8 9 8" /><path d="M5 10v10h14V10M9 20v-6h6v6" /></>,
  meetings: <><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M16 3v4M8 3v4M3 10h18" /></>,
  record: <><circle cx="12" cy="12" r="9" /><circle cx="12" cy="12" r="3.5" fill="currentColor" stroke="none" /></>,
  settings: <><circle cx="12" cy="12" r="3" /><path d="M19 15a2 2 0 0 0 .4 2.2l-2.2 2.2A2 2 0 0 0 15 19a2 2 0 0 0-1.2 1.8h-3.2A2 2 0 0 0 9 19a2 2 0 0 0-2.2.4l-2.2-2.2A2 2 0 0 0 5 15a2 2 0 0 0-1.8-1.2v-3.2A2 2 0 0 0 5 9a2 2 0 0 0-.4-2.2l2.2-2.2A2 2 0 0 0 9 5a2 2 0 0 0 1.2-1.8h3.2A2 2 0 0 0 15 5a2 2 0 0 0 2.2-.4l2.2 2.2A2 2 0 0 0 19 9a2 2 0 0 0 1.8 1.2v3.2A2 2 0 0 0 19 15Z" /></>,
  search: <><circle cx="11" cy="11" r="7" /><path d="m20 20-4-4" /></>,
  plus: <path d="M12 5v14M5 12h14" />,
  clock: <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>,
  sparkles: <><path d="m12 3 1.1 3.1L16 7.5l-2.9 1.4L12 12l-1.1-3.1L8 7.5l2.9-1.4L12 3Z" /><path d="m5.5 13 .8 2.2 2.2.8-2.2.8L5.5 19l-.8-2.2-2.2-.8 2.2-.8.8-2.2Z" /></>,
  shield: <><path d="M12 3 5 6v5c0 4.7 2.8 8.2 7 10 4.2-1.8 7-5.3 7-10V6l-7-3Z" /><path d="m9 12 2 2 4-4" /></>,
  headphones: <><path d="M4 14v-2a8 8 0 0 1 16 0v2M6 13H4a2 2 0 0 0-2 2v3a2 2 0 0 0 2 2h2v-7ZM18 13h2a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2h-2v-7Z" /></>,
  chevron: <path d="m9 18 6-6-6-6" />,
};
function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  return <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>;
}
const navItems = [
  { id: "home" as View, label: "Home", icon: "home" as IconName },
  { id: "meetings" as View, label: "Meetings", icon: "meetings" as IconName },
  { id: "record" as View, label: "Record", icon: "record" as IconName },
  { id: "settings" as View, label: "Settings", icon: "settings" as IconName },
];
const humanDate = (value: string) => new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
const durationLabel = (value: number | null) => value ? `${Math.max(1, Math.round(value / 60_000))} min` : "Not recorded";

function EmptyLibrary({ onRecord }: { onRecord: () => void }) {
  return <div className="empty-state"><div className="empty-visual" aria-hidden="true"><span className="paper back" /><span className="paper front"><i /><i /><i /><b><i /><i /><i /><i /><i /></b></span><span className="empty-spark one">✦</span><span className="empty-spark two">✦</span></div><h3>Your meeting library starts here</h3><p>Prepare a consented recording and it will be stored in your private local library.</p><button className="secondary-button" type="button" onClick={onRecord}><Icon name="record" size={17} /> Record your first meeting</button></div>;
}

function MeetingCard({ meeting, onChanged, onError }: { meeting: Meeting; onChanged: () => Promise<void>; onError: (value: string) => void }) {
  const run = async (action: () => Promise<unknown>) => { try { await action(); await onChanged(); } catch (error) { onError(String(error)); } };
  const rename = () => { const title = window.prompt("Meeting title", meeting.title); if (title?.trim() && title.trim() !== meeting.title) void run(() => meetingApi.rename(meeting.id, title)); };
  const editTags = () => { const value = window.prompt("Comma-separated tags", meeting.tags.join(", ")); if (value !== null) void run(() => meetingApi.replaceTags(meeting.id, value.split(","))); };
  const remove = () => { if (window.confirm(`Permanently delete “${meeting.title}” and all local data?`)) void run(() => meetingApi.delete(meeting.id)); };
  return <article className="meeting-card"><div className="meeting-card-main"><div className={`state-mark ${meeting.state}`}><Icon name={meeting.state === "ready" ? "sparkles" : "record"} size={18} /></div><div><div className="meeting-title-line"><h3>{meeting.title}</h3><span className={`state-badge ${meeting.state}`}>{meeting.state}</span></div><p>{humanDate(meeting.createdAt)} · {durationLabel(meeting.durationMs)}</p>{meeting.tags.length > 0 && <div className="tag-row">{meeting.tags.map((tag) => <span key={tag}>{tag}</span>)}</div>}</div></div><div className="meeting-actions">{meeting.state !== "archived" ? <><button type="button" onClick={rename}>Rename</button><button type="button" onClick={editTags}>Tags</button><button type="button" onClick={() => void run(() => meetingApi.archive(meeting.id))}>Archive</button></> : <><button type="button" onClick={() => void run(() => meetingApi.reopen(meeting.id))}>Reopen</button><button className="danger" type="button" onClick={remove}>Delete</button></>}</div></article>;
}

function RecordSetup({ onCreate, onError }: { onCreate: (title: string) => Promise<void>; onError: (value: string) => void }) {
  const [title, setTitle] = useState(""); const [consent, setConsent] = useState(false); const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => { event.preventDefault(); setBusy(true); try { await onCreate(title.trim() || `Meeting ${new Date().toLocaleString()}`); } catch (error) { onError(String(error)); } finally { setBusy(false); } };
  return <section className="page-view record-view"><div className="page-heading"><div><p className="eyebrow">NEW RECORDING</p><h1>Prepare your meeting</h1><p>Confirm consent before any audio capture begins.</p></div></div><form className="setup-card" onSubmit={(event) => void submit(event)}><label className="field"><span>Meeting title</span><input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="Weekly sync" /></label><div className="capture-summary"><Icon name="headphones" size={24} /><div><strong>Microphone + system audio</strong><span>WebRTC echo cancellation · headset recommended</span></div></div><label className="consent-check"><input type="checkbox" checked={consent} onChange={(event) => setConsent(event.target.checked)} /><span><strong>I confirm everyone has consented to this recording.</strong><small>You are responsible for following applicable laws and meeting policies.</small></span></label><button className="primary-button record-action" type="submit" disabled={!consent || busy}><Icon name="record" size={18} />{busy ? "Preparing…" : "Prepare recording"}</button></form></section>;
}

function SettingsView({ onSaved }: { onSaved: () => void }) {
  const [reminder, setReminder] = useState(() => localStorage.getItem("transcrip-it.headset-reminder") !== "false");
  const [retention, setRetention] = useState(() => localStorage.getItem("transcrip-it.retention") ?? "forever");
  const save = (event: FormEvent) => { event.preventDefault(); localStorage.setItem("transcrip-it.headset-reminder", String(reminder)); localStorage.setItem("transcrip-it.retention", retention); onSaved(); };
  return <section className="page-view"><div className="page-heading"><div><p className="eyebrow">LOCAL PREFERENCES</p><h1>Settings</h1><p>Control recording guidance and local retention.</p></div></div><form className="settings-card" onSubmit={save}><div className="setting-row"><div><strong>Headset reminder</strong><span>Show the audio-quality reminder before recording.</span></div><input aria-label="Headset reminder" type="checkbox" checked={reminder} onChange={(event) => setReminder(event.target.checked)} /></div><label className="setting-row"><div><strong>Keep meeting data</strong><span>Automatic retention will never remove data before this period.</span></div><select value={retention} onChange={(event) => setRetention(event.target.value)}><option value="forever">Until I delete it</option><option value="365">One year</option><option value="90">90 days</option><option value="30">30 days</option></select></label><div className="privacy-setting"><Icon name="shield" /><div><strong>Local processing is on</strong><span>Meeting content is not sent to a cloud service.</span></div></div><button className="primary-button" type="submit">Save settings</button></form></section>;
}

function App() {
  const [activeView, setActiveView] = useState<View>("home"); const [meetings, setMeetings] = useState<Meeting[]>([]); const [includeArchived, setIncludeArchived] = useState(false); const [search, setSearch] = useState(""); const [loading, setLoading] = useState(true); const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  const loadMeetings = useCallback(async () => { setLoading(true); try { setMeetings(await meetingApi.list(includeArchived)); setError(""); } catch (reason) { setError(String(reason)); } finally { setLoading(false); } }, [includeArchived]);
  useEffect(() => { void loadMeetings(); }, [loadMeetings]);
  const filtered = useMemo(() => { const query = search.trim().toLowerCase(); return query ? meetings.filter((meeting) => `${meeting.title} ${meeting.tags.join(" ")}`.toLowerCase().includes(query)) : meetings; }, [meetings, search]);
  const goToRecord = () => setActiveView("record");
  const createMeeting = async (title: string) => { const id = crypto.randomUUID(); await meetingApi.create({ id, idempotencyKey: `create-${id}`, title }); await loadMeetings(); setNotice("Meeting prepared locally. Recording controls are the next capture step."); setActiveView("meetings"); };
  const recordedMinutes = Math.round(meetings.reduce((sum, meeting) => sum + (meeting.durationMs ?? 0), 0) / 60_000);
  return <div className="app-shell">
    <aside className="sidebar"><div className="brand"><div className="brand-mark" aria-hidden="true"><span /><span /><span /><span /><span /></div><span>Transcrip<span className="brand-accent">·it</span></span></div><nav className="primary-nav" aria-label="Primary navigation">{navItems.map((item) => <button className={activeView === item.id ? "nav-item active" : "nav-item"} key={item.id} onClick={() => setActiveView(item.id)} type="button"><Icon name={item.icon} /><span>{item.label}</span></button>)}</nav><div className="sidebar-footer"><div className="local-status"><span className="status-dot" /><div><strong>Local workspace</strong><span>Your data stays on this device</span></div></div><div className="storage-row"><span>Meetings</span><span>{meetings.length} local</span></div><div className="storage-track"><span /></div></div></aside>
    <main className="main-panel"><header className="topbar"><div className="mobile-brand">Transcrip<span>·it</span></div><label className="search-box"><Icon name="search" size={18} /><span className="sr-only">Search meetings</span><input type="search" placeholder="Search meetings…" value={search} onChange={(event) => setSearch(event.target.value)} onFocus={() => setActiveView("meetings")} /><kbd>⌘ K</kbd></label><div className="privacy-pill"><Icon name="shield" size={16} /> Private by default</div><button className="avatar" type="button" aria-label="Local profile">JA</button></header>
      <div className="content">{error && <div className="message error" role="alert">{error}<button type="button" onClick={() => setError("")}>Dismiss</button></div>}{notice && <div className="message success" role="status">{notice}<button type="button" onClick={() => setNotice("")}>Dismiss</button></div>}
        {activeView === "home" && <><section className="hero-row"><div><p className="eyebrow">YOUR MEETING WORKSPACE</p><h1>Good morning</h1><p className="hero-copy">Capture the conversation. Keep the important parts.</p></div><button className="primary-button" type="button" onClick={goToRecord}><Icon name="plus" size={18} /> New recording</button></section><section className="metric-grid" aria-label="Workspace summary"><article className="metric-card"><div className="metric-icon violet"><Icon name="clock" /></div><div><span>MEETINGS</span><strong>{meetings.filter((meeting) => meeting.state !== "archived").length}</strong><small>Local</small></div></article><article className="metric-card"><div className="metric-icon blue"><Icon name="record" /></div><div><span>RECORDED</span><strong>{recordedMinutes}m</strong><small>All time</small></div></article><article className="metric-card"><div className="metric-icon amber"><Icon name="sparkles" /></div><div><span>READY</span><strong>{meetings.filter((meeting) => meeting.state === "ready").length}</strong><small>Meetings</small></div></article></section><section className="headset-card"><div className="headset-art"><Icon name="headphones" size={29} /></div><div className="headset-copy"><span className="recommended-badge">RECOMMENDED</span><h2>Use a headset for the clearest transcript</h2><p>A headset helps separate your voice from meeting audio and improves speaker accuracy.</p></div><button className="text-button" type="button" onClick={() => setActiveView("settings")}>Audio settings <Icon name="chevron" size={16} /></button></section><section className="section-block"><div className="section-heading"><div><h2>Recent meetings</h2><p>Your latest local meetings.</p></div><button className="text-button" type="button" onClick={() => setActiveView("meetings")}>View all <Icon name="chevron" size={16} /></button></div>{loading ? <p className="loading-copy">Loading your library…</p> : meetings.length === 0 ? <EmptyLibrary onRecord={goToRecord} /> : <div className="meeting-list">{meetings.slice(0, 3).map((meeting) => <MeetingCard key={meeting.id} meeting={meeting} onChanged={loadMeetings} onError={setError} />)}</div>}</section></>}
        {activeView === "meetings" && <section className="page-view"><div className="page-heading"><div><p className="eyebrow">LOCAL LIBRARY</p><h1>Meetings</h1><p>Organize, archive, and permanently remove recordings stored on this device.</p></div><button className="primary-button" type="button" onClick={goToRecord}><Icon name="plus" size={18} /> New recording</button></div><label className="archive-toggle"><input type="checkbox" checked={includeArchived} onChange={(event) => setIncludeArchived(event.target.checked)} /> Show archived meetings</label>{loading ? <p className="loading-copy">Loading your library…</p> : filtered.length === 0 ? <EmptyLibrary onRecord={goToRecord} /> : <div className="meeting-list">{filtered.map((meeting) => <MeetingCard key={meeting.id} meeting={meeting} onChanged={loadMeetings} onError={setError} />)}</div>}</section>}
        {activeView === "record" && <RecordSetup onCreate={createMeeting} onError={setError} />}{activeView === "settings" && <SettingsView onSaved={() => setNotice("Settings saved on this device.")} />}
      </div></main>
    <nav className="mobile-nav" aria-label="Mobile navigation">{navItems.map((item) => <button className={activeView === item.id ? "active" : ""} key={item.id} onClick={() => setActiveView(item.id)} type="button"><Icon name={item.icon} size={19} /><span>{item.label}</span></button>)}</nav>
  </div>;
}

export default App;
