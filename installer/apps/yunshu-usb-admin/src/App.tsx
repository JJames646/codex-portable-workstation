import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { formatBytes, progressPercent } from "./format";

type View = "home" | "write" | "build";
type Check = { code: string; label: string; passed: boolean; detail: string };
type Validation = {
  valid: boolean;
  root: string;
  buildId?: string;
  factory?: { schemaVersion: number; runtimeBuildId: string; fileCount: number };
  checks: Check[];
};
type Manifest = {
  productVersion: string;
  buildId: string;
  fileCount: number;
  expandedBytes: number;
};
type ManifestResult = { validation: Validation; manifest: Manifest };
type Disk = {
  root: string;
  label: string;
  filesystem: string;
  freeBytes: number;
  totalBytes: number;
  diskNumber: number;
  model: string;
  serial: string;
  eligible: boolean;
  reason?: string;
};
type Job = {
  jobId: string;
  kind: string;
  phase: string;
  message: string;
  target?: string;
  processedBytes: number;
  totalBytes: number;
  outputPath?: string;
  error?: string;
};

const TERMINAL_PHASES = new Set(["completed", "failed", "cancelled"]);

export default function App() {
  const [view, setView] = useState<View>("home");
  const [masterPath, setMasterPath] = useState("");
  const [version, setVersion] = useState("1.0.0");
  const [release, setRelease] = useState<ManifestResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [disks, setDisks] = useState<Disk[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [parallel, setParallel] = useState(2);
  const [jobs, setJobs] = useState<Record<string, Job>>({});
  const [outputDirectory, setOutputDirectory] = useState("");

  const activeJobIds = useMemo(
    () => Object.values(jobs).filter((job) => !TERMINAL_PHASES.has(job.phase)).map((job) => job.jobId),
    [jobs],
  );

  useEffect(() => {
    if (activeJobIds.length === 0) return;
    const timer = window.setInterval(() => {
      void Promise.all(
        activeJobIds.map((jobId) => invoke<Job>("admin_job_status", { jobId })),
      ).then((values) => {
        setJobs((current) => {
          const next = { ...current };
          for (const job of values) next[job.jobId] = job;
          return next;
        });
      });
    }, 800);
    return () => window.clearInterval(timer);
  }, [activeJobIds.join("|")]);

  async function chooseDirectory(setter: (value: string) => void) {
    const value = await open({ directory: true, multiple: false });
    if (typeof value === "string") setter(value);
  }

  async function validateRelease() {
    setBusy(true);
    setError("");
    try {
      const result = await invoke<ManifestResult>("admin_release_manifest_build", {
        masterPath,
        productVersion: version,
      });
      setRelease(result);
    } catch (cause) {
      setRelease(null);
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function refreshDisks() {
    if (!release) return;
    setBusy(true);
    setError("");
    try {
      const result = await invoke<Disk[]>("admin_usb_targets_list", {
        requiredBytes: release.manifest.expandedBytes,
        masterPath,
      });
      setDisks(result);
      setSelected((current) => current.filter((root) => result.some((disk) => disk.root === root && disk.eligible)));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function startBatch() {
    setBusy(true);
    setError("");
    try {
      const result = await invoke<{ batchId: string; jobIds: string[] }>("admin_batch_write_start", {
        masterPath,
        productVersion: version,
        targetRoots: selected,
        maxParallel: parallel,
      });
      const initial = await Promise.all(result.jobIds.map((jobId) => invoke<Job>("admin_job_status", { jobId })));
      setJobs(Object.fromEntries(initial.map((job) => [job.jobId, job])));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function buildInstaller() {
    setBusy(true);
    setError("");
    try {
      const jobId = await invoke<string>("admin_user_installer_build", {
        masterPath,
        productVersion: version,
        outputDirectory,
      });
      const job = await invoke<Job>("admin_job_status", { jobId });
      setJobs({ [job.jobId]: job });
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <button className="brand admin-brand" onClick={() => setView("home")}>
          <span className="brand-mark">云</span>
          <span><strong>云枢 AI</strong><small>管理员生产工具</small></span>
        </button>
        <nav className="primary-nav">
          <div className="nav-group">
            <button className={view === "home" ? "active" : ""} onClick={() => setView("home")}><span className="nav-icon">⌂</span><span><strong>生产首页</strong><small>生产任务总览</small></span></button>
            <button className={view === "write" ? "active" : ""} onClick={() => setView("write")}><span className="nav-icon">⇲</span><span><strong>批量制作</strong><small>直接写入移动盘</small></span></button>
            <button className={view === "build" ? "active" : ""} onClick={() => setView("build")}><span className="nav-icon">◆</span><span><strong>生成安装程序</strong><small>创建用户版安装包</small></span></button>
          </div>
          <div className="nav-group nav-group--bottom">
            <div className="sidebar-foot"><span className="portable-dot" /><span><strong>本地生产模式</strong><small>私钥不会交付给用户</small></span></div>
          </div>
        </nav>
      </aside>
      <div className="app-workspace">
        <header className="topbar">
          <div><div className="topbar-eyebrow">STAGE 11 · USB PRODUCTION</div><strong>{view === "home" ? "生产中心" : view === "write" ? "批量制作" : "生成安装程序"}</strong></div>
          <div className="privacy">仅管理员私下保存 · 不格式化目标盘</div>
        </header>

      {view === "home" ? (
        <section className="hero">
          <div className="eyebrow">STAGE 11 · USB PRODUCTION</div>
          <h1>选择一种生产方式</h1>
          <p>两种方式共用同一套清单、校验和安全写入核心，最终U盘内容保持一致。</p>
          <div className="entry-grid">
            <button className="entry-card" onClick={() => setView("write")}>
              <span className="entry-icon">⇲</span>
              <strong>直接写入 / 批量制作U盘</strong>
              <small>选择一个或多个USB移动盘，独立写入并逐文件校验。</small>
              <span className="entry-link">进入批量生产 →</span>
            </button>
            <button className="entry-card accent" onClick={() => setView("build")}>
              <span className="entry-icon">◆</span>
              <strong>生成用户安装程序</strong>
              <small>把完整母盘、签名清单和原生安装向导封装成用户版EXE。</small>
              <span className="entry-link">生成用户安装程序 →</span>
            </button>
          </div>
        </section>
      ) : (
        <section className="workspace">
          <button className="back" onClick={() => setView("home")}>← 返回</button>
          <div className="page-title">
            <div>
              <div className="eyebrow">{view === "write" ? "BATCH WRITER" : "INSTALLER BUILDER"}</div>
              <h1>{view === "write" ? "直接写入 / 批量制作U盘" : "生成用户安装程序"}</h1>
            </div>
          </div>

          <section className="panel">
            <h2>1. 选择并验证干净母盘</h2>
            <div className="form-row wide">
              <label>母盘文件夹<input value={masterPath} onChange={(event) => setMasterPath(event.target.value)} placeholder="请选择阶段十干净交付副本" /></label>
              <button className="secondary" onClick={() => void chooseDirectory(setMasterPath)}>选择文件夹</button>
            </div>
            <div className="form-row">
              <label>产品版本<input value={version} onChange={(event) => setVersion(event.target.value)} /></label>
              <button className="primary" disabled={busy || !masterPath || !version} onClick={() => void validateRelease()}>{busy ? "检查中…" : "验证母盘"}</button>
            </div>
            {release ? <ValidationSummary release={release} /> : null}
          </section>

          {view === "write" ? (
            <section className="panel">
              <div className="panel-heading"><h2>2. 选择目标U盘</h2><button className="secondary" disabled={!release || busy} onClick={() => void refreshDisks()}>刷新磁盘</button></div>
              <div className="disk-list">
                {disks.length === 0 ? <div className="empty">验证母盘后刷新USB磁盘列表</div> : disks.map((disk) => (
                  <label className={`disk ${disk.eligible ? "" : "disabled"}`} key={`${disk.diskNumber}-${disk.root}`}>
                    <input type="checkbox" disabled={!disk.eligible} checked={selected.includes(disk.root)} onChange={(event) => setSelected((current) => event.target.checked ? [...current, disk.root] : current.filter((root) => root !== disk.root))} />
                    <div><strong>{disk.root} {disk.label || "未命名U盘"}</strong><span>{disk.model} · {disk.filesystem} · 可用 {formatBytes(disk.freeBytes)} / {formatBytes(disk.totalBytes)}</span><small>{disk.eligible ? `物理磁盘 ${disk.diskNumber} · ${disk.serial || "无序列号"}` : disk.reason}</small></div>
                  </label>
                ))}
              </div>
              <div className="action-row">
                <label>并行数量<select value={parallel} onChange={(event) => setParallel(Number(event.target.value))}>{[1, 2, 3, 4].map((value) => <option key={value}>{value}</option>)}</select></label>
                <button className="primary danger-safe" disabled={!release || selected.length === 0 || busy || activeJobIds.length > 0} onClick={() => void startBatch()}>开始制作 {selected.length || ""} 个U盘</button>
              </div>
            </section>
          ) : (
            <section className="panel">
              <h2>2. 选择安装包输出目录</h2>
              <div className="form-row wide">
                <label>输出文件夹<input value={outputDirectory} onChange={(event) => setOutputDirectory(event.target.value)} placeholder="安装包和构建报告将保存在这里" /></label>
                <button className="secondary" onClick={() => void chooseDirectory(setOutputDirectory)}>选择文件夹</button>
              </div>
              <div className="notice">首次生成时会在管理员工具私有目录创建Ed25519私钥。请单独备份该私钥，不要交付给用户。</div>
              <div className="action-row right"><button className="primary" disabled={!release || !outputDirectory || busy || activeJobIds.length > 0} onClick={() => void buildInstaller()}>生成用户安装程序</button></div>
            </section>
          )}

          {Object.values(jobs).length > 0 ? <JobList jobs={Object.values(jobs)} /> : null}
          {error ? <div className="error-banner">{error}</div> : null}
        </section>
      )}
      </div>
    </main>
  );
}

function ValidationSummary({ release }: { release: ManifestResult }) {
  return <div className="validation"><div className="validation-head"><strong>母盘验证通过</strong><span>Build {release.manifest.buildId}</span><span>{release.manifest.fileCount.toLocaleString()} 文件</span><span>{formatBytes(release.manifest.expandedBytes)}</span></div><div className="checks">{release.validation.checks.map((check) => <div key={check.code} className={check.passed ? "pass" : "fail"}><b>{check.passed ? "✓" : "!"}</b><span>{check.label}<small>{check.detail}</small></span></div>)}</div></div>;
}

function JobList({ jobs }: { jobs: Job[] }) {
  return <section className="panel"><h2>生产任务</h2><div className="jobs">{jobs.map((job) => { const percent = progressPercent(job.processedBytes, job.totalBytes); return <div className="job" key={job.jobId}><div className="job-title"><strong>{job.target || "用户安装程序"}</strong><span data-phase={job.phase}>{job.phase}</span></div><div className="progress"><i style={{ width: `${percent}%` }} /></div><div className="job-meta"><span>{job.message}</span><span>{percent}% · {formatBytes(job.processedBytes)} / {formatBytes(job.totalBytes)}</span></div>{job.outputPath ? <code>{job.outputPath}</code> : null}{job.error ? <div className="job-error">{job.error}</div> : null}</div>; })}</div></section>;
}
