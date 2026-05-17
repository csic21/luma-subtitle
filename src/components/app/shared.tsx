import { type ReactNode } from "react";
import { AlertCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CardDescription, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useI18n } from "@/i18n";
import { downloadMeta, progressLabel, progressValue, statusText } from "@/lib/app-utils";
import { cn } from "@/lib/utils";
import type { DependencyInstallEvent, ModelDownloadEvent } from "@/types";

export function NoticeAlert({ message }: { message: string }) {
  const { t } = useI18n();
  if (!message) return null;
  return (
    <Alert className="notice-alert">
      <AlertCircle />
      <AlertTitle>{t("alert.notice")}</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

export function StatusBadge({ status, label }: { status: string; label?: string }) {
  const { t } = useI18n();
  return (
    <Badge variant="outline" className={cn("status-badge", `status-${status}`)}>
      {label ?? statusText(status, t)}
    </Badge>
  );
}

export function SectionTitle({
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

export function IconAction({
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

export function FieldBlock({
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

export function DownloadProgress({ event }: { event: ModelDownloadEvent | DependencyInstallEvent }) {
  const { t } = useI18n();
  const meta = downloadMeta(event, t);
  return (
    <div className={cn("download-card", `download-${event.status}`)}>
      <div className="progress-head compact">
        <span>{event.message}</span>
        <strong>{progressLabel(event.progress)}</strong>
      </div>
      <Progress className="hotdog-progress" value={progressValue(event.progress)} />
      {meta && <div className="download-meta">{meta}</div>}
    </div>
  );
}


