import { STAGE_LABEL, PROVIDER_STATE_LABEL } from "../../lib/format";
import type { HomeworkStage, ProviderState } from "../../lib/bridge";

const STAGE_TONE: Record<HomeworkStage, string> = { task: "", in_progress: "badge--info", ready: "badge--accent", submitted: "badge--ok" };

export function StageBadge({ stage }: { stage: HomeworkStage }) {
  return <span className={`badge ${STAGE_TONE[stage]}`}>{STAGE_LABEL[stage]}</span>;
}

const PROVIDER_DOT: Record<ProviderState, string> = {
  disconnected: "dot",
  connecting: "dot dot--info",
  connected: "dot dot--ok",
  error: "dot dot--danger",
  reauth_required: "dot dot--warn",
};

export function ProviderStateLabel({ state }: { state: ProviderState }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <span className={PROVIDER_DOT[state]} aria-hidden />
      <span>{PROVIDER_STATE_LABEL[state]}</span>
    </span>
  );
}
