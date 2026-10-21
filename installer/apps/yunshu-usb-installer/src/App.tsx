import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatBytes, progressPercent } from "./format";

type Step = "welcome" | "target" | "confirm" | "installing" | "completed";
type PackageInfo = { productVersion: string; buildId: string; fileCount: number; expandedBytes: number };
type Disk = {
  root: string;
  label: string;
  filesystem: string;
  freeBytes: number;
  totalBytes: number;
  model: string;
  serial: string;
  eligible: boolean;
  reason?: string;
};
type Job = {
  jobId: string;
  phase: string;
  message: string;
  targetRoot: string;
  processedBytes: number;
  totalBytes: number;
  error?: string;
};

const TERMINAL = new Set(["completed", "failed", "cancelled"]);

export default function App() {
  const [step, setStep] = useState<Step>("welcome");
  const [packageInfo, setPackageInfo] = useState<PackageInfo | null>(null);
  const [disks, setDisks] = useState<Disk[]>([]);
  const [selected, setSelected] = useState<Disk | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    void verifyPackage();
  }, []);

  useEffect(() => {
    if (!job || TERMINAL.has(job.phase)) return;
    const timer = window.setInterval(() => {
      void invoke<Job>("installer_install_status", { jobId: job.jobId }).then((next) => {
        setJob(next);
        if (next.phase === "completed") setStep("completed");
      });
    }, 700);
    return () => window.clearInterval(timer);
  }, [job?.jobId, job?.phase]);

  async function verifyPackage() {
    setBusy(true);
    setError("");
    try {
      setPackageInfo(await invoke<PackageInfo>("installer_self_verify"));
    } catch (cause) {
      setError(`安装包完整性校验失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  }

  async function loadTargets() {
    setBusy(true);
    setError("");
    try {
      const result = await invoke<Disk[]>("installer_usb_targets_list");
      setDisks(result);
      setSelected(null);
      setStep("target");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function startInstall() {
    if (!selected) return;
    setBusy(true);
    setError("");
    try {
      const jobId = await invoke<string>("installer_install_start", { targetRoot: selected.root });
      const current = await invoke<Job>("installer_install_status", { jobId });
      setJob(current);
      setStep("installing");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function cancelInstall() {
    if (!job) return;
    await invoke("installer_install_cancel", { jobId: job.jobId });
  }

  const percent = job ? progressPercent(job.processedBytes, job.totalBytes) : 0;

  return (
    <main className="app">
      <header><div className="logo">云枢</div><div><strong>云枢AI便携工作站</strong><span>一键写入U盘</span></div></header>
      <div className="steps">{["欢迎", "选择U盘", "确认", "安装", "完成"].map((label, index) => <div key={label} className={stepIndex(step) >= index ? "active" : ""}><i>{index + 1}</i><span>{label}</span></div>)}</div>

      <section className="card">
        {step === "welcome" ? <>
          <div className="hero-icon">⇩</div><h1>将云枢AI工作站安装到U盘</h1><p>安装器不会格式化U盘，也不会删除U盘上的其他文件。请选择一个NTFS或exFAT移动盘。</p>
          {packageInfo ? <div className="package-info"><span>版本 {packageInfo.productVersion}</span><span>Build {packageInfo.buildId}</span><span>{packageInfo.fileCount.toLocaleString()} 个文件</span><span>{formatBytes(packageInfo.expandedBytes)}</span></div> : null}
          <button className="primary" disabled={busy || !packageInfo} onClick={() => void loadTargets()}>{busy ? "正在验证安装包…" : "开始安装"}</button>
        </> : null}

        {step === "target" ? <>
          <div className="section-title"><div><h1>选择目标U盘</h1><p>系统盘、只读盘、FAT32和空间不足的磁盘会自动禁用。</p></div><button className="text-button" onClick={() => void loadTargets()}>刷新</button></div>
          <div className="disk-list">{disks.length === 0 ? <div className="empty">未发现USB移动盘，请插入U盘后刷新</div> : disks.map((disk) => <button key={disk.root} className={`disk ${selected?.root === disk.root ? "selected" : ""}`} disabled={!disk.eligible} onClick={() => setSelected(disk)}><i>{disk.root}</i><div><strong>{disk.label || "未命名U盘"}</strong><span>{disk.model} · {disk.filesystem}</span><small>{disk.eligible ? `可用 ${formatBytes(disk.freeBytes)} / ${formatBytes(disk.totalBytes)}` : disk.reason}</small></div><b>{selected?.root === disk.root ? "✓" : ""}</b></button>)}</div>
          <div className="actions"><button className="secondary" onClick={() => setStep("welcome")}>上一步</button><button className="primary" disabled={!selected} onClick={() => setStep("confirm")}>下一步</button></div>
        </> : null}

        {step === "confirm" && selected ? <>
          <div className="confirm-icon">!</div><h1>确认安装位置</h1><p>工作站将写入以下U盘。写入期间请勿拔出U盘或关闭电脑。</p>
          <div className="target-summary"><strong>{selected.root} {selected.label || "未命名U盘"}</strong><span>{selected.model} · {selected.filesystem}</span><span>预计写入 {formatBytes(packageInfo?.expandedBytes || 0)}</span></div>
          <div className="safety"><b>安装器会做什么</b><span>创建新的“云枢AI便携工作站”目录，并把正式交付文件放到U盘根目录。</span><b>安装器不会做什么</b><span>不会格式化、分区、清空U盘，也不会覆盖已存在的同名文件。</span></div>
          <div className="actions"><button className="secondary" onClick={() => setStep("target")}>重新选择</button><button className="primary" disabled={busy} onClick={() => void startInstall()}>确认并开始安装</button></div>
        </> : null}

        {step === "installing" && job ? <>
          <div className="spinner" /><h1>正在写入云枢AI工作站</h1><p>{job.message}</p>
          <div className="large-progress"><i style={{ width: `${percent}%` }} /></div><div className="progress-meta"><span>{formatBytes(job.processedBytes)} / {formatBytes(job.totalBytes)}</span><strong>{percent}%</strong></div>
          <div className="phase-list">{["preflight", "staging", "writing", "verifying", "committing"].map((phase) => <span key={phase} className={phaseReached(job.phase, phase) ? "done" : ""}>{phaseLabel(phase)}</span>)}</div>
          {job.error ? <div className="error">{job.error}</div> : null}<button className="secondary cancel" onClick={() => void cancelInstall()}>取消安装</button>
        </> : null}

        {step === "completed" ? <><div className="success">✓</div><h1>安装完成</h1><p>云枢AI便携工作站已完整写入 {job?.targetRoot}，文件校验全部通过，U盘窗口正在打开。</p><div className="complete-note">现在可以从U盘根目录打开环境助手，或进入“云枢AI便携工作站”启动工作站。</div></> : null}
        {error ? <div className="error">{error}</div> : null}
      </section>
      <footer>安装过程使用SHA-256与Ed25519完整性校验 · 不修改Windows系统设置</footer>
    </main>
  );
}

function stepIndex(step: Step) { return ["welcome", "target", "confirm", "installing", "completed"].indexOf(step); }
const phases = ["preflight", "staging", "writing", "verifying", "committing", "completed"];
function phaseReached(current: string, phase: string) { return phases.indexOf(current) >= phases.indexOf(phase); }
function phaseLabel(phase: string) { return ({ preflight: "目标预检", staging: "创建暂存", writing: "写入文件", verifying: "校验文件", committing: "提交安装" } as Record<string, string>)[phase]; }
