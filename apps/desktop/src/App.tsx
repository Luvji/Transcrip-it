import { useState, type ReactNode } from "react";
import "./App.css";

type IconName =
  | "home"
  | "meetings"
  | "record"
  | "settings"
  | "search"
  | "plus"
  | "clock"
  | "sparkles"
  | "shield"
  | "headphones"
  | "chevron"
  | "close";

const paths: Record<IconName, ReactNode> = {
  home: <><path d="m3 11 9-8 9 8" /><path d="M5 10v10h14V10M9 20v-6h6v6" /></>,
  meetings: <><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M16 3v4M8 3v4M3 10h18M8 14h.01M12 14h.01M16 14h.01M8 18h.01M12 18h.01" /></>,
  record: <><circle cx="12" cy="12" r="9" /><circle cx="12" cy="12" r="3.5" fill="currentColor" stroke="none" /></>,
  settings: <><circle cx="12" cy="12" r="3" /><path d="M19 15a2 2 0 0 0 .4 2.2l-2.2 2.2A2 2 0 0 0 15 19a2 2 0 0 0-1.2 1.8h-3.2A2 2 0 0 0 9 19a2 2 0 0 0-2.2.4l-2.2-2.2A2 2 0 0 0 5 15a2 2 0 0 0-1.8-1.2v-3.2A2 2 0 0 0 5 9a2 2 0 0 0-.4-2.2l2.2-2.2A2 2 0 0 0 9 5a2 2 0 0 0 1.2-1.8h3.2A2 2 0 0 0 15 5a2 2 0 0 0 2.2-.4l2.2 2.2A2 2 0 0 0 19 9a2 2 0 0 0 1.8 1.2v3.2A2 2 0 0 0 19 15Z" /></>,
  search: <><circle cx="11" cy="11" r="7" /><path d="m20 20-4-4" /></>,
  plus: <path d="M12 5v14M5 12h14" />,
  clock: <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>,
  sparkles: <><path d="m12 3 1.1 3.1L16 7.5l-2.9 1.4L12 12l-1.1-3.1L8 7.5l2.9-1.4L12 3Z" /><path d="m5.5 13 .8 2.2 2.2.8-2.2.8L5.5 19l-.8-2.2-2.2-.8 2.2-.8.8-2.2ZM18.5 13l.6 1.6 1.4.7-1.4.7-.6 1.5-.6-1.5-1.4-.7 1.4-.7.6-1.6Z" /></>,
  shield: <><path d="M12 3 5 6v5c0 4.7 2.8 8.2 7 10 4.2-1.8 7-5.3 7-10V6l-7-3Z" /><path d="m9 12 2 2 4-4" /></>,
  headphones: <><path d="M4 14v-2a8 8 0 0 1 16 0v2M6 13H4a2 2 0 0 0-2 2v3a2 2 0 0 0 2 2h2v-7ZM18 13h2a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2h-2v-7Z" /></>,
  chevron: <path d="m9 18 6-6-6-6" />,
  close: <path d="m6 6 12 12M18 6 6 18" />,
};

function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      {paths[name]}
    </svg>
  );
}

const navItems = [
  { id: "home", label: "Home", icon: "home" as IconName },
  { id: "meetings", label: "Meetings", icon: "meetings" as IconName },
  { id: "record", label: "Record", icon: "record" as IconName },
  { id: "settings", label: "Settings", icon: "settings" as IconName },
];

function App() {
  const [activeView, setActiveView] = useState("home");
  const [showNotice, setShowNotice] = useState(false);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true"><span /><span /><span /><span /><span /></div>
          <span>Transcrip<span className="brand-accent">·it</span></span>
        </div>

        <nav className="primary-nav" aria-label="Primary navigation">
          {navItems.map((item) => (
            <button
              className={activeView === item.id ? "nav-item active" : "nav-item"}
              key={item.id}
              onClick={() => setActiveView(item.id)}
              type="button"
            >
              <Icon name={item.icon} />
              <span>{item.label}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-footer">
          <div className="local-status">
            <span className="status-dot" />
            <div><strong>Local workspace</strong><span>Your data stays on this device</span></div>
          </div>
          <div className="storage-row"><span>Storage</span><span>0 MB used</span></div>
          <div className="storage-track"><span /></div>
        </div>
      </aside>

      <main className="main-panel">
        <header className="topbar">
          <div className="mobile-brand">Transcrip<span>·it</span></div>
          <label className="search-box">
            <Icon name="search" size={18} />
            <span className="sr-only">Search meetings</span>
            <input type="search" placeholder="Search meetings…" />
            <kbd>⌘ K</kbd>
          </label>
          <div className="privacy-pill"><Icon name="shield" size={16} /> Private by default</div>
          <button className="avatar" type="button" aria-label="Open profile">JA</button>
        </header>

        <div className="content">
          <section className="hero-row">
            <div>
              <p className="eyebrow">YOUR MEETING WORKSPACE</p>
              <h1>Good morning</h1>
              <p className="hero-copy">Capture the conversation. Keep the important parts.</p>
            </div>
            <button className="primary-button" type="button" onClick={() => setShowNotice(true)}>
              <Icon name="plus" size={18} /> New recording
            </button>
          </section>

          <section className="metric-grid" aria-label="Workspace summary">
            <article className="metric-card">
              <div className="metric-icon violet"><Icon name="clock" /></div>
              <div><span>MEETINGS</span><strong>0</strong><small>All time</small></div>
            </article>
            <article className="metric-card">
              <div className="metric-icon blue"><Icon name="record" /></div>
              <div><span>RECORDED</span><strong>0m</strong><small>This week</small></div>
            </article>
            <article className="metric-card">
              <div className="metric-icon amber"><Icon name="sparkles" /></div>
              <div><span>ACTION ITEMS</span><strong>0</strong><small>Ready to review</small></div>
            </article>
          </section>

          <section className="headset-card">
            <div className="headset-art"><Icon name="headphones" size={29} /></div>
            <div className="headset-copy">
              <span className="recommended-badge">RECOMMENDED</span>
              <h2>Use a headset for the clearest transcript</h2>
              <p>A headset helps separate your voice from meeting audio and improves speaker accuracy.</p>
            </div>
            <button className="text-button" type="button" onClick={() => setActiveView("settings")}>Audio settings <Icon name="chevron" size={16} /></button>
          </section>

          <section className="section-block">
            <div className="section-heading">
              <div><h2>Recent meetings</h2><p>Your latest recordings will appear here.</p></div>
              <button className="text-button" type="button" onClick={() => setActiveView("meetings")}>View all <Icon name="chevron" size={16} /></button>
            </div>
            <div className="empty-state">
              <div className="empty-visual" aria-hidden="true">
                <span className="paper back" />
                <span className="paper front"><i /><i /><i /><b><i /><i /><i /><i /><i /></b></span>
                <span className="empty-spark one">✦</span><span className="empty-spark two">✦</span>
              </div>
              <h3>Your meeting library starts here</h3>
              <p>Record a meeting and Transcrip-it will turn it into a searchable transcript, summary, and action list.</p>
              <button className="secondary-button" type="button" onClick={() => setShowNotice(true)}><Icon name="record" size={17} /> Record your first meeting</button>
            </div>
          </section>
        </div>
      </main>

      <nav className="mobile-nav" aria-label="Mobile navigation">
        {navItems.map((item) => (
          <button className={activeView === item.id ? "active" : ""} key={item.id} onClick={() => setActiveView(item.id)} type="button">
            <Icon name={item.icon} size={19} /><span>{item.label}</span>
          </button>
        ))}
      </nav>

      {showNotice && (
        <div className="dialog-backdrop" role="presentation" onMouseDown={() => setShowNotice(false)}>
          <section className="notice-dialog" role="dialog" aria-modal="true" aria-labelledby="notice-title" onMouseDown={(event) => event.stopPropagation()}>
            <button className="dialog-close" type="button" aria-label="Close" onClick={() => setShowNotice(false)}><Icon name="close" /></button>
            <div className="dialog-icon"><Icon name="record" size={25} /></div>
            <p className="eyebrow">COMING NEXT</p>
            <h2 id="notice-title">The workspace is ready</h2>
            <p>The recording workflow will be connected in the next milestone. Your local-first desktop shell is up and running.</p>
            <button className="primary-button full" type="button" onClick={() => setShowNotice(false)}>Got it</button>
          </section>
        </div>
      )}
    </div>
  );
}

export default App;
