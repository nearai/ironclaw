const qs = (id) => document.getElementById(id);

const dom = {
  generatedAt: qs("generatedAt"),
  kpiOpenclaw: qs("kpiOpenclaw"),
  kpiGateway: qs("kpiGateway"),
  kpiSandbox: qs("kpiSandbox"),
  kpiDmScope: qs("kpiDmScope"),
  checksList: qs("checksList"),
  handoffTitle: qs("handoffTitle"),
  handoffSummary: qs("handoffSummary"),
  handoffNext: qs("handoffNext"),
  commandsList: qs("commandsList"),
  eventsList: qs("eventsList"),
  logTail: qs("logTail"),
  artifactList: qs("artifactList"),
  statusPreview: qs("statusPreview"),
  deepStatusPreview: qs("deepStatusPreview"),
  refreshBtn: qs("refreshDataBtn"),
};

const badgeClass = (status) => {
  if (status === "ok") return "status-pill status-ok";
  if (status === "warn") return "status-pill status-warn";
  return "status-pill status-bad";
};

function renderList(node, items, renderItem) {
  node.innerHTML = "";
  if (!items || !items.length) {
    const li = document.createElement("li");
    li.textContent = "none";
    node.appendChild(li);
    return;
  }
  items.forEach((item) => node.appendChild(renderItem(item)));
}

function render(data) {
  dom.generatedAt.textContent = `snapshot: ${data.generated_at_utc}`;
  dom.kpiOpenclaw.textContent = data.runtime.openclaw_cli_present ? "available" : "missing";
  dom.kpiGateway.textContent = data.config_snapshot.gateway_mode;
  dom.kpiSandbox.textContent = data.config_snapshot.sandbox_mode;
  dom.kpiDmScope.textContent = data.config_snapshot.dm_scope;

  renderList(dom.checksList, data.checks, (check) => {
    const li = document.createElement("li");
    const badge = document.createElement("span");
    badge.className = badgeClass(check.status);
    badge.textContent = check.status.toUpperCase();
    const text = document.createElement("span");
    text.textContent = `${check.label}: ${check.detail}`;
    li.appendChild(badge);
    li.appendChild(text);
    return li;
  });

  dom.handoffTitle.textContent = data.handoff.title || "-";
  dom.handoffSummary.textContent = data.handoff.summary || "-";

  renderList(dom.handoffNext, data.handoff.next_actions, (line) => {
    const li = document.createElement("li");
    li.textContent = line;
    return li;
  });

  renderList(dom.commandsList, data.quick_commands, (cmd) => {
    const li = document.createElement("li");
    const label = document.createElement("strong");
    label.textContent = cmd.label;
    const code = document.createElement("code");
    code.className = "cmd";
    code.textContent = cmd.command;
    li.appendChild(label);
    li.appendChild(code);
    return li;
  });

  renderList(dom.eventsList, data.events, (event) => {
    const li = document.createElement("li");
    li.innerHTML = `<strong>${event.ts}</strong> • ${event.agent} • ${event.title}`;
    return li;
  });

  renderList(dom.artifactList, data.artifacts, (artifact) => {
    const li = document.createElement("li");
    li.innerHTML = `<strong>${artifact.label}</strong><div class="cmd">${artifact.path}</div>`;
    return li;
  });

  dom.logTail.textContent = data.runtime.preflight_log_tail.join("\n") || "no log";
  dom.statusPreview.textContent = data.runtime.status_preview.join("\n") || "n/a";
  dom.deepStatusPreview.textContent = data.runtime.deep_status_preview.join("\n") || "n/a";
}

async function loadSnapshot() {
  const res = await fetch(`./data/status.json?t=${Date.now()}`);
  if (!res.ok) throw new Error(`failed to load snapshot: ${res.status}`);
  const data = await res.json();
  render(data);
}

async function init() {
  try {
    await loadSnapshot();
  } catch (err) {
    dom.generatedAt.textContent = String(err);
  }

  dom.refreshBtn.addEventListener("click", async () => {
    dom.generatedAt.textContent = "refreshing... run refresh script in terminal if data is stale";
    await loadSnapshot();
  });
}

init();
