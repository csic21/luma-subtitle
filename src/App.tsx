import { type ReactNode, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Database,
  Download,
  ExternalLink,
  Eye,
  FileVideo,
  FolderOpen,
  KeyRound,
  Languages,
  Loader2,
  Play,
  Plus,
  RefreshCw,
  Save,
  Settings,
  Subtitles,
  Terminal,
  Trash2,
  Upload,
} from "lucide-react";
import {
  HashRouter,
  Link,
  Navigate,
  Route,
  Routes,
  useNavigate,
  useParams,
} from "react-router-dom";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import lumaLogoMark from "@/assets/luma-logo-mark.png";

type SettingsState = {
  base_url: string;
  model: string;
  temperature: number;
  translation_shard_size: number;
  whisper_model_path: string;
  whisper_language: string;
  target_language: string;
  has_api_key: boolean;
};

type EnvironmentState = {
  ffmpeg_path: string | null;
  whisper_path: string | null;
  gpu_name: string | null;
  cuda_driver: string | null;
  resource_dir: string;
  config_dir: string;
  sidecar_dir: string;
  model_dir: string;
};

type JobOutputs = {
  source_file_name: string;
  translated_file_name?: string | null;
  output_dir: string;
  segment_count: number;
};

type JobEvent = {
  job_id: string;
  stage: string;
  status: "running" | "completed" | "failed" | "cancelled";
  message: string;
  progress: number;
  outputs?: JobOutputs | null;
  error?: string | null;
};

type ModelDownloadEvent = {
  preset_id: string;
  file_name: string;
  status: "running" | "completed" | "failed";
  message: string;
  progress: number;
  path?: string | null;
  error?: string | null;
  bytes_per_second?: number | null;
  eta_seconds?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
};

type DependencyInstallEvent = {
  item: string;
  status: "running" | "completed" | "failed";
  message: string;
  progress: number;
  path?: string | null;
  error?: string | null;
  bytes_per_second?: number | null;
  eta_seconds?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
};

type DownloadStatus = {
  model?: ModelDownloadEvent | null;
  dependency?: DependencyInstallEvent | null;
};

type TaskSettingsSnapshot = {
  output_dir?: string | null;
  target_language: string;
  whisper_model_path: string;
  whisper_language: string;
  base_url: string;
  model: string;
  temperature: number;
  translation_shard_size: number;
};

type TaskRecord = {
  id: string;
  source_type: "video" | "srt";
  video_path?: string | null;
  srt_path?: string | null;
  file_name: string;
  status: string;
  stage: string;
  message: string;
  progress: number;
  settings: TaskSettingsSnapshot;
  source_srt_path?: string | null;
  translated_srt_path?: string | null;
  source_file_name?: string | null;
  translated_file_name?: string | null;
  output_dir?: string | null;
  segment_count?: number | null;
  exported_source_srt?: string | null;
  exported_translated_srt?: string | null;
  exported_output_dir?: string | null;
  error?: string | null;
  created_at: number;
  updated_at: number;
};

type SubtitlePreview = {
  source_srt: string;
  translated_srt?: string | null;
  source_file_name: string;
  translated_file_name?: string | null;
};

type QueueSettings = {
  max_concurrency: number;
};

type WhisperModelPreset = {
  id: string;
  label: string;
  fileName: string;
};

type WhisperLanguageOption = {
  value: string;
  label: string;
};

const defaultSettings: SettingsState = {
  base_url: "https://api.openai.com",
  model: "gpt-4o-mini",
  temperature: 0.2,
  translation_shard_size: 200,
  whisper_model_path: "",
  whisper_language: "auto",
  target_language: "简体中文",
  has_api_key: false,
};

const languageOptions = [
  "简体中文",
  "繁体中文",
  "English",
  "日本語",
  "한국어",
  "Deutsch",
  "Français",
  "Español",
];

const whisperLanguageOptions: WhisperLanguageOption[] = [
  { value: "auto", label: "自动检测" },
  { value: "zh", label: "中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
  { value: "ko", label: "한국어" },
  { value: "de", label: "Deutsch" },
  { value: "fr", label: "Français" },
  { value: "es", label: "Español" },
  { value: "it", label: "Italiano" },
  { value: "pt", label: "Português" },
  { value: "ru", label: "Русский" },
];

const whisperModelPresets: WhisperModelPreset[] = [
  {
    id: "tiny",
    label: "tiny · 75 MiB · 最快",
    fileName: "ggml-tiny.bin",
  },
  {
    id: "base",
    label: "base · 142 MiB · 快速测试",
    fileName: "ggml-base.bin",
  },
  {
    id: "small",
    label: "small · 466 MiB · 更准",
    fileName: "ggml-small.bin",
  },
  {
    id: "large-v3-turbo-q5_0",
    label: "large-v3-turbo-q5_0 · 547 MiB · 3060 Ti 推荐",
    fileName: "ggml-large-v3-turbo-q5_0.bin",
  },
];

function fileName(path?: string | null) {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

function progressValue(progress: number) {
  return Math.round(Math.max(0, Math.min(1, progress)) * 100);
}

function progressLabel(progress: number) {
  return `${progressValue(progress)}%`;
}

function bytesLabel(bytes?: number | null) {
  if (!bytes || bytes <= 0) return "";
  const mib = bytes / (1024 * 1024);
  if (mib >= 1) return `${mib.toFixed(1)} MiB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
}

function speedLabel(bytesPerSecond?: number | null) {
  const bytes = bytesLabel(bytesPerSecond);
  return bytes ? `${bytes}/s` : "";
}

function etaLabel(seconds?: number | null) {
  if (seconds === null || seconds === undefined) return "";
  if (seconds <= 0) return "即将完成";
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes <= 0) return `剩余 ${rest}s`;
  return `剩余 ${minutes}m ${rest}s`;
}

function downloadMeta(event?: ModelDownloadEvent | DependencyInstallEvent | null) {
  if (!event) return "";
  const parts: string[] = [];
  const downloaded = bytesLabel(event.downloaded_bytes);
  const total = bytesLabel(event.total_bytes);
  const speed = speedLabel(event.bytes_per_second);
  const eta = etaLabel(event.eta_seconds);

  if (downloaded && total) parts.push(`${downloaded} / ${total}`);
  else if (downloaded) parts.push(downloaded);
  if (speed) parts.push(speed);
  if (eta && event.status === "running") parts.push(eta);
  return parts.join(" · ");
}

function statusText(status?: string) {
  if (status === "created") return "已创建";
  if (status === "queued") return "排队中";
  if (status === "running") return "进行中";
  if (status === "completed") return "已完成";
  if (status === "exported") return "已导出";
  if (status === "failed") return "失败";
  if (status === "cancelled") return "已取消";
  if (status === "interrupted") return "已中断";
  return "待开始";
}

function stageText(stage?: string) {
  const labels: Record<string, string> = {
    created: "等待操作",
    transcribe: "转写",
    extracting: "抽取音频",
    transcribing: "本地转写",
    "source-srt": "生成原文",
    "source-ready": "原文就绪",
    "preparing-translation": "准备翻译",
    "translate-shards": "分片翻译",
    "translate-shard": "分片翻译",
    "render-translated-srt": "生成译文",
    exporting: "导出",
    exported: "已导出",
    completed: "已完成",
    failed: "失败",
    cancelled: "已取消",
    interrupted: "已中断",
  };
  return stage ? labels[stage] ?? stage : "待开始";
}

function isTranslateStage(stage?: string) {
  return Boolean(
    stage &&
      (stage.includes("translate") ||
        stage.includes("translation") ||
        stage === "render-translated-srt"),
  );
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function taskBusy(task: TaskRecord) {
  return task.status === "queued" || task.status === "running";
}

function canRunOperation(task: TaskRecord, operation: "transcribe" | "translate" | "export") {
  if (taskBusy(task)) return false;
  if (operation === "transcribe") return task.source_type === "video";
  if (operation === "translate") return Boolean(task.source_srt_path);
  return Boolean(task.source_srt_path);
}

function formattedTime(seconds: number) {
  if (!seconds) return "—";
  return new Date(seconds * 1000).toLocaleString();
}

function operationLabel(operation: "transcribe" | "translate" | "export") {
  if (operation === "transcribe") return "转写";
  if (operation === "translate") return "翻译";
  return "导出";
}

function NoticeAlert({ message }: { message: string }) {
  if (!message) return null;
  return (
    <Alert className="notice-alert">
      <AlertCircle />
      <AlertTitle>提示</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

function StatusBadge({ status, label }: { status: string; label?: string }) {
  return (
    <Badge variant="outline" className={cn("status-badge", `status-${status}`)}>
      {label ?? statusText(status)}
    </Badge>
  );
}

function SectionTitle({
  icon,
  title,
  description,
}: {
  icon: ReactNode;
  title: string;
  description?: string;
}) {
  return (
    <div className="section-title">
      <span className="section-icon">{icon}</span>
      <div>
        <CardTitle>{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </div>
    </div>
  );
}

function IconAction({
  label,
  children,
  ...props
}: React.ComponentProps<typeof Button> & { label: string; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button size="icon-sm" variant="ghost" aria-label={label} {...props}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function FieldBlock({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="form-field">
      <Label>{label}</Label>
      {children}
    </div>
  );
}

function DownloadProgress({ event }: { event: ModelDownloadEvent | DependencyInstallEvent }) {
  return (
    <div className={cn("download-card", `download-${event.status}`)}>
      <div className="progress-head compact">
        <span>{event.message}</span>
        <strong>{progressLabel(event.progress)}</strong>
      </div>
      <Progress className="hotdog-progress" value={progressValue(event.progress)} />
      {downloadMeta(event) && <div className="download-meta">{downloadMeta(event)}</div>}
    </div>
  );
}

export default function App() {
  return (
    <TooltipProvider delayDuration={180}>
      <HashRouter>
        <AppLayout />
      </HashRouter>
    </TooltipProvider>
  );
}

function AppLayout() {
  return (
    <main className="app-shell">
      <header className="topbar">
        <Link to="/tasks" className="brand-link">
          <span className="brand-mark" aria-hidden="true">
            <img src={lumaLogoMark} alt="" />
          </span>
          <span className="brand-copy">
            <strong>Luma Subtitle</strong>
            <span>批量视频转写、翻译与 SRT 导出</span>
          </span>
        </Link>
        <Button asChild variant="secondary" size="icon-lg" title="设置">
          <Link to="/settings" aria-label="设置">
            <Settings />
          </Link>
        </Button>
      </header>

      <Routes>
        <Route path="/" element={<Navigate to="/tasks" replace />} />
        <Route path="/tasks" element={<TasksPage />} />
        <Route path="/tasks/:taskId" element={<TaskDetailPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<Navigate to="/tasks" replace />} />
      </Routes>
    </main>
  );
}

function TasksPage() {
  const navigate = useNavigate();
  const [tasks, setTasks] = useState<TaskRecord[]>([]);
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [queueSettings, setQueueSettings] = useState<QueueSettings>({ max_concurrency: 2 });
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [outputDir, setOutputDir] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!hasTauriRuntime()) {
      setNotice("请在 Tauri 应用中运行，以使用本地任务队列。");
      return;
    }
    void refreshTasks();
    void refreshSettings();
    void refreshQueueSettings();

    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;
    let unlistenDeleted: (() => void) | undefined;
    listen<TaskRecord>("task-updated", (event) => {
      setTasks((current) => upsertTask(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    });
    listen<JobEvent>("job-event", (event) => {
      setTasks((current) => applyJobEventToTasks(current, event.payload));
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
    });
    listen<string>("task-deleted", (event) => {
      setTasks((current) => current.filter((task) => task.id !== event.payload));
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(event.payload);
        return next;
      });
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenDeleted = fn;
    });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenJob?.();
      unlistenDeleted?.();
    };
  }, []);

  useEffect(() => {
    if (!hasTauriRuntime() || !tasks.some(taskBusy)) return;
    const timer = window.setInterval(() => {
      void refreshTasks();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [tasks]);

  async function refreshTasks() {
    try {
      setTasks(await invoke<TaskRecord[]>("list_tasks"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshSettings() {
    try {
      const loaded = await invoke<SettingsState>("load_settings");
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshQueueSettings() {
    try {
      setQueueSettings(await invoke<QueueSettings>("load_queue_settings"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function saveConcurrency(maxConcurrency: number) {
    try {
      const saved = await invoke<QueueSettings>("save_queue_settings", { maxConcurrency });
      setQueueSettings(saved);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function pickOutputDir() {
    try {
      const picked = await invoke<string | null>("select_output_dir");
      if (picked) setOutputDir(picked);
    } catch (error) {
      setNotice(`选择输出目录失败：${errorText(error)}`);
    }
  }

  async function createVideoTask() {
    try {
      const picked = await invoke<string | null>("select_video");
      if (!picked) return;
      const created = await invoke<TaskRecord>("create_video_task", {
        request: taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked),
      });
      setTasks((current) => upsertTask(current, created));
      setNotice(`已创建任务：${created.file_name}`);
    } catch (error) {
      setNotice(`创建任务失败：${errorText(error)}`);
    }
  }

  async function createSrtTask() {
    try {
      const picked = await invoke<string | null>("select_srt");
      if (!picked) return;
      const created = await invoke<TaskRecord>("create_srt_task", {
        request: taskCreatePayload({ ...settings, output_dir: outputDir || null }, picked, "srt"),
      });
      setTasks((current) => upsertTask(current, created));
      setNotice(`已导入 SRT：${created.file_name}`);
    } catch (error) {
      setNotice(`导入 SRT 失败：${errorText(error)}`);
    }
  }

  async function runOperation(taskId: string, operation: "transcribe" | "translate" | "export") {
    try {
      await invoke("run_task_operation", { taskId, operation });
      await refreshTasks();
      setNotice(`${operationLabel(operation)}已加入队列`);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function runSelected(operation: "transcribe" | "translate" | "export") {
    const taskIds = tasks
      .filter((task) => selectedIds.has(task.id) && canRunOperation(task, operation))
      .map((task) => task.id);
    if (taskIds.length === 0) {
      setNotice("没有可执行的选中任务");
      return;
    }
    try {
      await invoke("run_task_operations", { taskIds, operation });
      await refreshTasks();
      setNotice(`已将 ${taskIds.length} 个任务加入${operationLabel(operation)}队列`);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function cancelTask(taskId: string) {
    try {
      await invoke("cancel_task", { taskId });
      await refreshTasks();
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function cancelSelected() {
    const taskIds = tasks.filter((task) => selectedIds.has(task.id) && taskBusy(task)).map((task) => task.id);
    await Promise.all(taskIds.map((taskId) => cancelTask(taskId)));
  }

  async function deleteTask(taskId: string) {
    try {
      await invoke("delete_task", { taskId });
      setTasks((current) => current.filter((task) => task.id !== taskId));
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(taskId);
        return next;
      });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  const allSelected = tasks.length > 0 && tasks.every((task) => selectedIds.has(task.id));
  const busyCount = tasks.filter(taskBusy).length;
  const doneCount = tasks.filter((task) => task.status === "completed" || task.status === "exported").length;
  const failedCount = tasks.filter((task) => task.status === "failed").length;

  function toggleTask(taskId: string) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  }

  function toggleAll() {
    setSelectedIds((current) => {
      if (tasks.length > 0 && tasks.every((task) => current.has(task.id))) return new Set();
      return new Set(tasks.map((task) => task.id));
    });
  }

  return (
    <>
      <NoticeAlert message={notice} />

      <section className="metric-grid">
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>全部任务</CardDescription>
            <CardTitle>{tasks.length}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>进行中</CardDescription>
            <CardTitle>{busyCount}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>已完成</CardDescription>
            <CardTitle>{doneCount}</CardTitle>
          </CardHeader>
        </Card>
        <Card size="sm" className="metric-card">
          <CardHeader>
            <CardDescription>失败</CardDescription>
            <CardTitle>{failedCount}</CardTitle>
          </CardHeader>
        </Card>
      </section>

      <Card className="toolbar-card">
        <CardContent className="toolbar-content">
          <div className="toolbar-main">
            <Button onClick={createVideoTask} title="创建视频任务">
              <Plus data-icon="inline-start" />
              新建视频
            </Button>
            <Button variant="secondary" onClick={createSrtTask} title="导入原文 SRT">
              <Upload data-icon="inline-start" />
              导入 SRT
            </Button>
            <Button variant="secondary" onClick={pickOutputDir} title="选择输出目录">
              <FolderOpen data-icon="inline-start" />
              输出目录
            </Button>
            <code className="path-chip">{outputDir || "默认输出到素材所在目录"}</code>
          </div>

          <Separator className="toolbar-separator" />

          <div className="toolbar-actions">
            <div className="concurrency-field">
              <Label htmlFor="max-concurrency">并发</Label>
              <Input
                id="max-concurrency"
                type="number"
                min="1"
                max="4"
                value={queueSettings.max_concurrency}
                onChange={(event) => void saveConcurrency(Number.parseInt(event.target.value, 10) || 1)}
              />
            </div>
            <Button variant="secondary" onClick={() => runSelected("transcribe")}>
              <Play data-icon="inline-start" />
              转写
            </Button>
            <Button variant="secondary" onClick={() => runSelected("translate")}>
              <Languages data-icon="inline-start" />
              翻译
            </Button>
            <Button variant="secondary" onClick={() => runSelected("export")}>
              <Download data-icon="inline-start" />
              导出
            </Button>
            <Button variant="destructive" onClick={cancelSelected}>
              <CircleStop data-icon="inline-start" />
              取消
            </Button>
            <IconAction label="刷新任务" onClick={refreshTasks}>
              <RefreshCw />
            </IconAction>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <SectionTitle icon={<Database />} title="任务队列" description="选择任务后可批量转写、翻译或导出" />
          <CardAction>
            <Badge variant="secondary">{tasks.length} 条</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="table-card-content">
          <Table className="task-table">
            <TableHeader>
              <TableRow>
                <TableHead className="select-cell">
                  <Checkbox checked={allSelected} onCheckedChange={toggleAll} aria-label="选择全部任务" />
                </TableHead>
                <TableHead>文件</TableHead>
                <TableHead>状态</TableHead>
                <TableHead>进度</TableHead>
                <TableHead>目标语言</TableHead>
                <TableHead>输出目录</TableHead>
                <TableHead>更新时间</TableHead>
                <TableHead>操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tasks.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8}>
                    <div className="empty-state">暂无任务</div>
                  </TableCell>
                </TableRow>
              ) : (
                tasks.map((task) => (
                  <TableRow key={task.id} data-state={taskBusy(task) ? "selected" : undefined}>
                    <TableCell className="select-cell">
                      <Checkbox
                        checked={selectedIds.has(task.id)}
                        onCheckedChange={() => toggleTask(task.id)}
                        aria-label={`选择 ${task.file_name}`}
                      />
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        className="file-button"
                        onClick={() => navigate(`/tasks/${task.id}`)}
                      >
                        <FileVideo data-icon="inline-start" />
                        <span>{task.file_name}</span>
                      </Button>
                      <small>{task.source_type === "srt" ? "SRT" : fileName(task.video_path)}</small>
                    </TableCell>
                    <TableCell>
                      <StatusBadge status={task.status} />
                      <small>{stageText(task.stage)}</small>
                    </TableCell>
                    <TableCell>
                      <div className="table-progress-stack">
                        <Progress className="hotdog-progress" value={progressValue(task.progress)} />
                        <small>{task.message}</small>
                      </div>
                    </TableCell>
                    <TableCell>{task.settings.target_language}</TableCell>
                    <TableCell>
                      <code className="table-code">{task.output_dir || task.settings.output_dir || "同素材目录"}</code>
                    </TableCell>
                    <TableCell>{formattedTime(task.updated_at)}</TableCell>
                    <TableCell>
                      <div className="row-actions">
                        <IconAction
                          label="转写"
                          onClick={() => runOperation(task.id, "transcribe")}
                          disabled={!canRunOperation(task, "transcribe")}
                        >
                          <Play />
                        </IconAction>
                        <IconAction
                          label="翻译"
                          onClick={() => runOperation(task.id, "translate")}
                          disabled={!canRunOperation(task, "translate")}
                        >
                          <Languages />
                        </IconAction>
                        <IconAction
                          label="导出"
                          onClick={() => runOperation(task.id, "export")}
                          disabled={!canRunOperation(task, "export")}
                        >
                          <Download />
                        </IconAction>
                        <IconAction label="取消" onClick={() => cancelTask(task.id)} disabled={!taskBusy(task)}>
                          <CircleStop />
                        </IconAction>
                        <IconAction label="详情" onClick={() => navigate(`/tasks/${task.id}`)}>
                          <Eye />
                        </IconAction>
                        <IconAction
                          label="删除"
                          onClick={() => deleteTask(task.id)}
                          disabled={taskBusy(task)}
                          className="danger-action"
                        >
                          <Trash2 />
                        </IconAction>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </>
  );
}

function TaskDetailPage() {
  const { taskId = "" } = useParams();
  const navigate = useNavigate();
  const [task, setTask] = useState<TaskRecord | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<TaskSettingsSnapshot | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [subtitlePreview, setSubtitlePreview] = useState<SubtitlePreview | null>(null);
  const [subtitleView, setSubtitleView] = useState<"translated" | "source">("source");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!taskId) return;
    if (!hasTauriRuntime()) {
      setNotice("请在 Tauri 应用中运行，以查看任务详情。");
      return;
    }
    void refreshTask();
    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenJob: (() => void) | undefined;
    listen<TaskRecord>("task-updated", (event) => {
      if (event.payload.id !== taskId) return;
      setTask(event.payload);
      setSettingsDraft(normalizeTaskSettings(event.payload.settings));
      void refreshLogs();
      if (event.payload.source_srt_path) void refreshPreview(event.payload);
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenTask = fn;
    });
    listen<JobEvent>("job-event", (event) => {
      if (event.payload.job_id !== taskId) return;
      setTask((current) => (current ? applyJobEventToTask(current, event.payload) : current));
      setLogs((current) => appendRealtimeLog(current, event.payload));
      if (event.payload.status !== "running" || event.payload.outputs) {
        void refreshTask();
      }
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlistenJob = fn;
    });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenJob?.();
    };
  }, [taskId]);

  useEffect(() => {
    if (!hasTauriRuntime() || !taskId || !task || !taskBusy(task)) return;
    const timer = window.setInterval(() => {
      void refreshTask({ preview: false });
    }, 1500);
    return () => window.clearInterval(timer);
  }, [taskId, task?.status]);

  async function refreshTask(options: { preview?: boolean } = {}) {
    try {
      const loaded = await invoke<TaskRecord>("get_task", { taskId });
      setTask(loaded);
      setSettingsDraft(normalizeTaskSettings(loaded.settings));
      await refreshLogs();
      if ((options.preview ?? true) && loaded.source_srt_path) await refreshPreview(loaded);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshLogs() {
    try {
      setLogs(await invoke<string[]>("get_task_logs", { taskId }));
    } catch {
      setLogs([]);
    }
  }

  async function refreshPreview(currentTask = task) {
    if (!currentTask?.source_srt_path) return;
    try {
      const preview = await invoke<SubtitlePreview>("subtitle_preview", { jobId: taskId });
      setSubtitlePreview(preview);
      setSubtitleView(preview.translated_srt?.trim() ? "translated" : "source");
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function runOperation(operation: "transcribe" | "translate" | "export") {
    try {
      await invoke("run_task_operation", { taskId, operation });
      await refreshTask();
      setNotice(`${operationLabel(operation)}已加入队列`);
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function cancelTask() {
    try {
      await invoke("cancel_task", { taskId });
      await refreshTask();
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function applyCurrentSettingsToTask() {
    try {
      const updated = await invoke<TaskRecord>("apply_current_settings_to_task", { taskId });
      setTask(updated);
      setSettingsDraft(normalizeTaskSettings(updated.settings));
      await refreshLogs();
      setNotice("已从全局设置导入到此任务");
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function saveTaskSettings() {
    if (!settingsDraft) return;
    try {
      const updated = await invoke<TaskRecord>("update_task_settings", {
        taskId,
        settings: taskSettingsUpdatePayload(settingsDraft),
      });
      setTask(updated);
      setSettingsDraft(normalizeTaskSettings(updated.settings));
      await refreshLogs();
      setNotice("任务配置已保存");
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function pickTaskWhisperModel() {
    try {
      const picked = await invoke<string | null>("select_whisper_model");
      if (picked) setSettingsDraft((current) => (current ? { ...current, whisper_model_path: picked } : current));
    } catch (error) {
      setNotice(`选择 Whisper 模型失败：${errorText(error)}`);
    }
  }

  async function openOutputDir() {
    const target = task?.exported_output_dir || task?.output_dir || task?.settings.output_dir;
    if (!target) return;
    try {
      await invoke("open_path", { path: target });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  const flowSteps = useMemo(() => {
    if (!task) return [];
    return [
      {
        label: "素材",
        detail: task.file_name,
        state: "done",
      },
      {
        label: "转写",
        detail: task.source_srt_path ? "原文字幕已生成" : task.message,
        state: task.source_srt_path ? "done" : taskBusy(task) ? "active" : "idle",
      },
      {
        label: "翻译",
        detail: task.translated_srt_path ? "译文已生成" : "等待翻译",
        state: task.translated_srt_path ? "done" : isTranslateStage(task.stage) ? "active" : "idle",
      },
      {
        label: "导出",
        detail: task.exported_output_dir ? "文件已写入目录" : "等待导出",
        state: task.exported_output_dir ? "done" : task.stage === "exporting" ? "active" : "idle",
      },
    ];
  }, [task]);

  const hasTranslatedSubtitle = Boolean(subtitlePreview?.translated_srt?.trim());
  const activeSubtitleBody =
    subtitleView === "translated" && hasTranslatedSubtitle
      ? subtitlePreview?.translated_srt
      : subtitlePreview?.source_srt;
  const activeSubtitleFileName =
    subtitleView === "translated" && hasTranslatedSubtitle
      ? subtitlePreview?.translated_file_name
      : subtitlePreview?.source_file_name;
  const taskSettingsDirty = Boolean(
    task &&
      settingsDraft &&
      JSON.stringify(normalizeTaskSettings(task.settings)) !== JSON.stringify(normalizeTaskSettings(settingsDraft)),
  );

  if (!task) {
    return (
      <>
        <NoticeAlert message={notice} />
        <Card className="loading-panel">
          <CardContent>正在加载任务...</CardContent>
        </Card>
      </>
    );
  }
  const taskConfig = settingsDraft ?? normalizeTaskSettings(task.settings);

  return (
    <>
      <section className="page-heading">
        <Button variant="secondary" onClick={() => navigate("/tasks")}>
          <ArrowLeft data-icon="inline-start" />
          返回队列
        </Button>
        <div>
          <h1>{task.file_name}</h1>
          <p>{task.message}</p>
        </div>
        <StatusBadge status={task.status} />
      </section>

      <NoticeAlert message={notice} />

      <section className="flow-strip" aria-label="处理流程">
        {flowSteps.map((step, index) => (
          <div className={cn("flow-step", step.state)} key={step.label}>
            <span className="flow-index">{step.state === "done" ? <Check /> : index + 1}</span>
            <div>
              <strong>{step.label}</strong>
              <span>{step.detail}</span>
            </div>
            {index < flowSteps.length - 1 && <ChevronRight className="flow-arrow" />}
          </div>
        ))}
      </section>

      <section className="workspace">
        <div className="left-column">
          <Card>
            <CardHeader>
              <SectionTitle icon={<FileVideo />} title="任务" />
            </CardHeader>
            <CardContent className="stack-panel">
              <div className="detail-list">
                <span>素材</span>
                <code>{task.video_path || task.srt_path || task.file_name}</code>
                <span>输出目录</span>
                <code>{task.output_dir || task.settings.output_dir || "同素材目录"}</code>
                <span>更新时间</span>
                <code>{formattedTime(task.updated_at)}</code>
                <span>字幕段数</span>
                <code>{task.segment_count ?? "—"}</code>
              </div>
              {task.error && (
                <Alert variant="destructive">
                  <AlertCircle />
                  <AlertTitle>任务错误</AlertTitle>
                  <AlertDescription>{task.error}</AlertDescription>
                </Alert>
              )}
              <div className="action-row">
                <Button
                  variant="secondary"
                  onClick={() => runOperation("transcribe")}
                  disabled={!canRunOperation(task, "transcribe")}
                >
                  <Play data-icon="inline-start" />
                  转写
                </Button>
                <Button
                  variant="secondary"
                  onClick={() => runOperation("translate")}
                  disabled={!canRunOperation(task, "translate")}
                >
                  <Languages data-icon="inline-start" />
                  翻译
                </Button>
                <Button onClick={() => runOperation("export")} disabled={!canRunOperation(task, "export")}>
                  <Download data-icon="inline-start" />
                  导出
                </Button>
                <Button variant="destructive" onClick={cancelTask} disabled={!taskBusy(task)}>
                  <CircleStop data-icon="inline-start" />
                  取消
                </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Settings />} title="任务配置" />
              <CardAction>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={applyCurrentSettingsToTask}
                  disabled={taskBusy(task)}
                  title="从全局设置导入"
                >
                  <RefreshCw data-icon="inline-start" />
                  从全局导入
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent className="settings-form">
              <FieldBlock label="Whisper 模型">
                <div className="input-action">
                  <Input
                    value={taskConfig.whisper_model_path}
                    onChange={(event) =>
                      setSettingsDraft({ ...taskConfig, whisper_model_path: event.target.value })
                    }
                    disabled={taskBusy(task)}
                    placeholder="未设置"
                    title={taskConfig.whisper_model_path || "选择 Whisper 模型"}
                  />
                  <IconAction label="选择 Whisper 模型" onClick={pickTaskWhisperModel} disabled={taskBusy(task)}>
                    <FolderOpen />
                  </IconAction>
                </div>
              </FieldBlock>

              <div className="grid-two">
                <FieldBlock label="源语言">
                  <Select
                    value={taskConfig.whisper_language || "auto"}
                    onValueChange={(value) => setSettingsDraft({ ...taskConfig, whisper_language: value })}
                    disabled={taskBusy(task)}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {whisperLanguageOptions.map((language) => (
                          <SelectItem key={language.value} value={language.value}>
                            {language.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </FieldBlock>
                <FieldBlock label="目标语言">
                  <Select
                    value={taskConfig.target_language}
                    onValueChange={(value) => setSettingsDraft({ ...taskConfig, target_language: value })}
                    disabled={taskBusy(task)}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {languageOptions.map((language) => (
                          <SelectItem key={language} value={language}>
                            {language}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </FieldBlock>
              </div>

              <div className="grid-two">
                <FieldBlock label="Base URL">
                  <Input
                    value={taskConfig.base_url}
                    onChange={(event) => setSettingsDraft({ ...taskConfig, base_url: event.target.value })}
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
                <FieldBlock label="翻译模型">
                  <Input
                    value={taskConfig.model}
                    onChange={(event) => setSettingsDraft({ ...taskConfig, model: event.target.value })}
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
              </div>

              <div className="grid-two">
                <FieldBlock label="Temperature">
                  <Input
                    type="number"
                    min="0"
                    max="1"
                    step="0.1"
                    value={taskConfig.temperature}
                    onChange={(event) =>
                      setSettingsDraft({ ...taskConfig, temperature: Number.parseFloat(event.target.value) || 0 })
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
                <FieldBlock label="每片字幕条数">
                  <Input
                    type="number"
                    min="1"
                    max="1000"
                    step="1"
                    value={taskConfig.translation_shard_size ?? defaultSettings.translation_shard_size}
                    onChange={(event) =>
                      setSettingsDraft({
                        ...taskConfig,
                        translation_shard_size:
                          Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                      })
                    }
                    disabled={taskBusy(task)}
                  />
                </FieldBlock>
              </div>

              <div className="action-row end">
                <Button
                  variant="secondary"
                  onClick={() => setSettingsDraft(normalizeTaskSettings(task.settings))}
                  disabled={taskBusy(task) || !taskSettingsDirty}
                >
                  撤销
                </Button>
                <Button onClick={saveTaskSettings} disabled={taskBusy(task) || !taskSettingsDirty}>
                  <Save data-icon="inline-start" />
                  保存任务配置
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="right-column">
          <Card className={cn("progress-panel", `progress-${task.status}`)}>
            <CardHeader>
              <SectionTitle
                icon={
                  task.status === "completed" || task.status === "exported" ? (
                    <CheckCircle2 />
                  ) : taskBusy(task) ? (
                    <Loader2 className="spin" />
                  ) : (
                    <Terminal />
                  )
                }
                title="进度"
              />
              <CardAction>
                <StatusBadge status={task.status} label={stageText(task.stage)} />
              </CardAction>
            </CardHeader>
            <CardContent className="stack-panel">
              <div className="progress-head">
                <span>{task.message}</span>
                <strong>{progressLabel(task.progress)}</strong>
              </div>
              <Progress className="hotdog-progress large" value={progressValue(task.progress)} />
              {(task.source_file_name || task.translated_file_name) && (
                <div className="outputs">
                  <Button onClick={() => runOperation("export")} disabled={!canRunOperation(task, "export")}>
                    <Download data-icon="inline-start" />
                    导出字幕
                  </Button>
                  {task.source_file_name && <code>{task.source_file_name}</code>}
                  {task.translated_file_name && <code>{task.translated_file_name}</code>}
                  {task.exported_output_dir && (
                    <Button variant="secondary" onClick={openOutputDir} title="打开导出目录">
                      <ExternalLink data-icon="inline-start" />
                      打开导出目录
                    </Button>
                  )}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Subtitles />} title="字幕预览" />
              <CardAction>
                <Button variant="secondary" size="sm" onClick={() => refreshPreview()}>
                  <RefreshCw data-icon="inline-start" />
                  刷新
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent>
              <Tabs
                value={subtitleView}
                onValueChange={(value) => setSubtitleView(value as "translated" | "source")}
                className="subtitle-tabs"
              >
                <TabsList>
                  <TabsTrigger value="source">原文</TabsTrigger>
                  {hasTranslatedSubtitle && <TabsTrigger value="translated">译文</TabsTrigger>}
                </TabsList>
              </Tabs>

              {subtitlePreview ? (
                <>
                  <code className="subtitle-file">{activeSubtitleFileName}</code>
                  <ScrollArea className="subtitle-preview">
                    <pre>{activeSubtitleBody || "暂无字幕内容"}</pre>
                  </ScrollArea>
                </>
              ) : (
                <div className="subtitle-empty">字幕完成后显示在这里</div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <SectionTitle icon={<Terminal />} title="日志" />
            </CardHeader>
            <CardContent>
              <ScrollArea className="log-list">
                {logs.length === 0 ? (
                  <span className="muted">暂无任务日志</span>
                ) : (
                  logs.map((line, index) => <p key={`${line}-${index}`}>{line}</p>)
                )}
              </ScrollArea>
            </CardContent>
          </Card>
        </div>
      </section>
    </>
  );
}

function SettingsPage() {
  const navigate = useNavigate();
  const [settings, setSettings] = useState<SettingsState>(defaultSettings);
  const [apiKey, setApiKey] = useState("");
  const [env, setEnv] = useState<EnvironmentState | null>(null);
  const [notice, setNotice] = useState("");
  const [whisperPresetId, setWhisperPresetId] = useState(whisperModelPresets[1].id);
  const [modelDownload, setModelDownload] = useState<ModelDownloadEvent | null>(null);
  const [dependencyInstall, setDependencyInstall] = useState<DependencyInstallEvent | null>(null);

  const selectedWhisperPreset =
    whisperModelPresets.find((preset) => preset.id === whisperPresetId) ?? whisperModelPresets[0];
  const modelDownloading = modelDownload?.status === "running";
  const dependencyInstalling = dependencyInstall?.status === "running";
  const hasApiCredential = settings.has_api_key || apiKey.trim().length > 0;
  const environmentReady = Boolean(env?.ffmpeg_path && env?.whisper_path);

  const envRows = useMemo(() => {
    if (!env) return [];
    return [
      ["FFmpeg", env.ffmpeg_path ?? "未找到"],
      ["whisper.cpp", env.whisper_path ?? "未找到"],
      ["GPU", env.gpu_name ? `${env.gpu_name}${env.cuda_driver ? ` / ${env.cuda_driver}` : ""}` : "未检测到"],
      ["依赖目录", env.sidecar_dir],
      ["模型目录", env.model_dir],
      ["资源目录", env.resource_dir],
      ["配置目录", env.config_dir],
    ];
  }, [env]);

  useEffect(() => {
    if (!hasTauriRuntime()) {
      setNotice("请在 Tauri 应用中运行，以使用本地配置。");
      return;
    }

    void refreshSettings();
    void refreshEnvironment();
    void refreshDownloadStatus();

    let unlistenModelDownload: (() => void) | undefined;
    let unlistenDependencyInstall: (() => void) | undefined;
    listen<ModelDownloadEvent>("model-download-event", (event) => {
      setModelDownload(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      unlistenModelDownload = fn;
    });
    listen<DependencyInstallEvent>("dependency-install-event", (event) => {
      setDependencyInstall(event.payload);
      if (event.payload.status === "failed") setNotice(event.payload.error ?? event.payload.message);
    }).then((fn) => {
      unlistenDependencyInstall = fn;
    });
    return () => {
      unlistenModelDownload?.();
      unlistenDependencyInstall?.();
    };
  }, []);

  useEffect(() => {
    if (!modelDownloading && !dependencyInstalling) return;
    const timer = window.setInterval(() => {
      void refreshDownloadStatus();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [modelDownloading, dependencyInstalling]);

  async function refreshSettings() {
    try {
      const loaded = await invoke<SettingsState>("load_settings");
      setSettings({ ...defaultSettings, ...loaded });
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshEnvironment() {
    try {
      setEnv(await invoke<EnvironmentState>("check_environment"));
    } catch (error) {
      setNotice(errorText(error));
    }
  }

  async function refreshDownloadStatus() {
    try {
      const status = await invoke<DownloadStatus>("download_status");
      if (status.model) setModelDownload(status.model);
      if (status.dependency) setDependencyInstall(status.dependency);
    } catch {
      // Event updates still work if polling is unavailable in an older backend.
    }
  }

  async function saveSettings(showNotice = true) {
    try {
      const saved = await invoke<SettingsState>("save_settings", {
        payload: {
          ...settings,
          api_key: apiKey,
        },
      });
      setSettings(saved);
      if (showNotice) setNotice("设置已保存，新建任务会使用这份配置");
    } catch (error) {
      if (showNotice) setNotice(`保存设置失败：${errorText(error)}`);
      throw error;
    }
  }

  async function pickWhisperModel() {
    try {
      const picked = await invoke<string | null>("select_whisper_model");
      if (picked) setSettings((current) => ({ ...current, whisper_model_path: picked }));
    } catch (error) {
      setNotice(`选择 Whisper 模型失败：${errorText(error)}`);
    }
  }

  async function downloadWhisperPreset() {
    setNotice("");
    setModelDownload({
      preset_id: selectedWhisperPreset.id,
      file_name: selectedWhisperPreset.fileName,
      status: "running",
      message: "准备下载模型",
      progress: 0,
    });

    try {
      const modelPath = await invoke<string>("download_whisper_model", {
        request: { preset_id: selectedWhisperPreset.id },
      });
      const nextSettings = { ...settings, whisper_model_path: modelPath };
      setSettings(nextSettings);
      const saved = await invoke<SettingsState>("save_settings", {
        payload: {
          ...nextSettings,
          api_key: apiKey,
        },
      });
      setSettings(saved);
      setModelDownload((current) => ({
        ...(current ?? {}),
        preset_id: selectedWhisperPreset.id,
        file_name: selectedWhisperPreset.fileName,
        status: "completed",
        message: "模型已下载",
        progress: 1,
        path: modelPath,
        error: null,
      }));
      setNotice(`已下载并选择 ${fileName(modelPath)}`);
    } catch (error) {
      setModelDownload((current) =>
        current
          ? {
              ...current,
              status: "failed",
              message: "模型下载失败",
              progress: 0,
              error: String(error),
            }
          : null,
      );
      setNotice(String(error));
    }
  }

  async function installDependencies() {
    setNotice("");
    setDependencyInstall({
      item: "依赖",
      status: "running",
      message: "准备安装依赖",
      progress: 0,
    });

    try {
      const installedPaths = await invoke<string[]>("install_dependencies");
      await refreshEnvironment();
      setDependencyInstall((current) => ({
        item: current?.item ?? "依赖",
        ...(current ?? {}),
        status: "completed",
        message: "依赖已安装",
        progress: 1,
        path: installedPaths[installedPaths.length - 1] ?? current?.path ?? null,
        error: null,
      }));
      setNotice("FFmpeg 和 whisper.cpp 已安装");
    } catch (error) {
      setDependencyInstall((current) =>
        current
          ? {
              ...current,
              status: "failed",
              message: "依赖安装失败",
              progress: 0,
              error: String(error),
            }
          : null,
      );
      setNotice(String(error));
    }
  }

  async function openManagedDir(path?: string | null) {
    if (!path) return;
    await invoke("open_path", { path });
  }

  return (
    <>
      <section className="page-heading">
        <Button variant="secondary" onClick={() => navigate("/tasks")}>
          <ArrowLeft data-icon="inline-start" />
          返回队列
        </Button>
        <div>
          <h1>设置</h1>
          <p>全局配置只影响之后新建的任务</p>
        </div>
      </section>

      <NoticeAlert message={notice} />

      <section className="settings-page-grid">
        <Card>
          <CardHeader>
            <SectionTitle icon={<Settings />} title="模型与接口" description="配置本地转写模型和翻译接口" />
          </CardHeader>
          <CardContent className="settings-form">
            <FieldBlock label="Whisper 模型">
              <div className="input-action">
                <Input
                  value={settings.whisper_model_path ? fileName(settings.whisper_model_path) : ""}
                  readOnly
                  placeholder={selectedWhisperPreset.fileName}
                  onClick={pickWhisperModel}
                  title={settings.whisper_model_path || "选择 Whisper 模型"}
                />
                <IconAction label="选择 Whisper 模型" onClick={pickWhisperModel}>
                  <FolderOpen />
                </IconAction>
              </div>
            </FieldBlock>

            <FieldBlock label="模型预设">
              <div className="input-action">
                <Select value={whisperPresetId} onValueChange={setWhisperPresetId}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {whisperModelPresets.map((preset) => (
                        <SelectItem key={preset.id} value={preset.id}>
                          {preset.label}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
                <IconAction
                  label={`下载并选择 ${selectedWhisperPreset.fileName}`}
                  onClick={downloadWhisperPreset}
                  disabled={modelDownloading}
                >
                  {modelDownloading ? <Loader2 className="spin" /> : <Download />}
                </IconAction>
              </div>
            </FieldBlock>

            {modelDownload && <DownloadProgress event={modelDownload} />}

            <div className="grid-two">
              <FieldBlock label="源语言">
                <Select
                  value={settings.whisper_language}
                  onValueChange={(value) => setSettings({ ...settings, whisper_language: value })}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {whisperLanguageOptions.map((language) => (
                        <SelectItem key={language.value} value={language.value}>
                          {language.label}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </FieldBlock>
              <FieldBlock label="目标语言">
                <Select
                  value={settings.target_language}
                  onValueChange={(value) => setSettings({ ...settings, target_language: value })}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {languageOptions.map((language) => (
                        <SelectItem key={language} value={language}>
                          {language}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </FieldBlock>
            </div>

            <div className="grid-two">
              <FieldBlock label="Base URL">
                <Input
                  value={settings.base_url}
                  onChange={(event) => setSettings({ ...settings, base_url: event.target.value })}
                />
              </FieldBlock>
              <FieldBlock label="翻译模型">
                <Input value={settings.model} onChange={(event) => setSettings({ ...settings, model: event.target.value })} />
              </FieldBlock>
            </div>

            <div className="grid-two">
              <FieldBlock label="API Key">
                <div className="key-field">
                  <KeyRound />
                  <Input
                    type="password"
                    value={apiKey}
                    onChange={(event) => setApiKey(event.target.value)}
                    placeholder={settings.has_api_key ? "已保存" : "未保存"}
                  />
                </div>
              </FieldBlock>
              <FieldBlock label="Temperature">
                <Input
                  type="number"
                  min="0"
                  max="1"
                  step="0.1"
                  value={settings.temperature}
                  onChange={(event) =>
                    setSettings({ ...settings, temperature: Number.parseFloat(event.target.value) || 0 })
                  }
                />
              </FieldBlock>
            </div>

            <FieldBlock label="每片字幕条数">
              <Input
                type="number"
                min="1"
                max="1000"
                step="1"
                value={settings.translation_shard_size}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    translation_shard_size:
                      Number.parseInt(event.target.value, 10) || defaultSettings.translation_shard_size,
                  })
                }
              />
            </FieldBlock>

            <Alert className={cn("credential-alert", hasApiCredential ? "ready" : "warn")}>
              {hasApiCredential ? <CheckCircle2 /> : <AlertCircle />}
              <AlertTitle>{hasApiCredential ? "翻译接口已可用" : "翻译接口待配置"}</AlertTitle>
              <AlertDescription>
                {hasApiCredential ? "新建任务会使用当前接口配置。" : "翻译前需要填写或保存 API Key。"}
              </AlertDescription>
            </Alert>

            <div className="action-row end">
              <Button variant="secondary" onClick={() => saveSettings()} title="保存设置">
                <Save data-icon="inline-start" />
                保存
              </Button>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionTitle icon={<Terminal />} title="环境" description="本地依赖、模型和资源目录" />
            <CardAction>
              <StatusBadge status={environmentReady ? "ready" : "warn"} label={environmentReady ? "依赖就绪" : "需安装"} />
            </CardAction>
          </CardHeader>
          <CardContent className="stack-panel">
            <div className="env-table">
              {envRows.map(([name, value]) => (
                <div className="env-row" key={name}>
                  <span>{name}</span>
                  <code>{value}</code>
                </div>
              ))}
            </div>
            <div className="action-row end">
              <Button variant="secondary" onClick={() => refreshEnvironment()}>
                <RefreshCw data-icon="inline-start" />
                刷新
              </Button>
              <Button variant="secondary" onClick={() => openManagedDir(env?.sidecar_dir)} disabled={!env?.sidecar_dir}>
                <FolderOpen data-icon="inline-start" />
                打开目录
              </Button>
              <Button variant="secondary" onClick={installDependencies} disabled={dependencyInstalling}>
                {dependencyInstalling ? <Loader2 data-icon="inline-start" className="spin" /> : <Download data-icon="inline-start" />}
                下载到依赖目录
              </Button>
            </div>
            {dependencyInstall && <DownloadProgress event={dependencyInstall} />}
          </CardContent>
        </Card>
      </section>
    </>
  );
}

function upsertTask(tasks: TaskRecord[], incoming: TaskRecord) {
  const exists = tasks.some((task) => task.id === incoming.id);
  const next = exists ? tasks.map((task) => (task.id === incoming.id ? incoming : task)) : [incoming, ...tasks];
  return next.sort((a, b) => b.updated_at - a.updated_at || b.created_at - a.created_at);
}

function applyJobEventToTask(task: TaskRecord, event: JobEvent): TaskRecord {
  if (task.id !== event.job_id) return task;
  const outputs = event.outputs ?? null;
  return {
    ...task,
    status: event.status,
    stage: event.stage,
    message: event.message,
    progress: event.progress,
    source_file_name: outputs?.source_file_name ?? task.source_file_name,
    translated_file_name: outputs?.translated_file_name ?? task.translated_file_name,
    output_dir: outputs?.output_dir ?? task.output_dir,
    segment_count: outputs?.segment_count ?? task.segment_count,
    error: event.error ?? null,
    updated_at: Math.floor(Date.now() / 1000),
  };
}

function applyJobEventToTasks(tasks: TaskRecord[], event: JobEvent) {
  let changed = false;
  const next = tasks.map((task) => {
    if (task.id !== event.job_id) return task;
    changed = true;
    return applyJobEventToTask(task, event);
  });
  return changed ? next.sort((a, b) => b.updated_at - a.updated_at || b.created_at - a.created_at) : tasks;
}

function appendRealtimeLog(logs: string[], event: JobEvent) {
  const lines = [`${event.stage} · ${event.message}`];
  if (event.error) lines.push(`error · ${event.error}`);
  const next = [...logs];
  for (const line of lines) {
    if (!next.includes(line)) next.push(line);
  }
  return next;
}

function normalizeTaskSettings(settings: TaskSettingsSnapshot): TaskSettingsSnapshot {
  return {
    ...settings,
    translation_shard_size: settings.translation_shard_size ?? defaultSettings.translation_shard_size,
  };
}

function taskSettingsUpdatePayload(settings: TaskSettingsSnapshot) {
  return {
    target_language: settings.target_language,
    whisper_model_path: settings.whisper_model_path,
    whisper_language: settings.whisper_language,
    base_url: settings.base_url,
    model: settings.model,
    temperature: settings.temperature,
    translation_shard_size: settings.translation_shard_size ?? defaultSettings.translation_shard_size,
  };
}

function taskCreatePayload(
  settings: SettingsState & { output_dir?: string | null },
  path: string,
  sourceType: "video" | "srt" = "video",
) {
  return {
    [sourceType === "video" ? "video_path" : "srt_path"]: path,
    output_dir: settings.output_dir || null,
    target_language: settings.target_language,
    whisper_model_path: settings.whisper_model_path,
    whisper_language: settings.whisper_language,
    base_url: settings.base_url,
    model: settings.model,
    temperature: settings.temperature,
    translation_shard_size: settings.translation_shard_size,
  };
}
