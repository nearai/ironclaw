// IronClaw Legal — single-page client.
// Talks to /api/skills/legal/* with a bearer token stored in localStorage.
// No build step, no dependencies; vanilla JS.
(() => {
  "use strict";

  const TOKEN_KEY = "ironclaw_legal_token";
  const API_BASE = "/api/skills/legal";

  const $ = (id) => document.getElementById(id);

  // --- State ---
  const state = {
    token: localStorage.getItem(TOKEN_KEY) || "",
    view: "projects",         // 'projects' | 'project' | 'chat'
    project: null,            // {id, name, documents?}
    chat: null,               // {id, title?, messages?}
  };

  // --- Toasts ---
  const toast = (msg, kind = "info") => {
    const el = $("toast");
    el.textContent = msg;
    el.className = "toast" + (kind === "error" ? " error" : "");
    el.hidden = false;
    clearTimeout(toast._t);
    toast._t = setTimeout(() => { el.hidden = true; }, 4000);
  };

  // --- HTTP helpers ---
  const headers = (extra = {}) => ({
    Authorization: "Bearer " + state.token,
    Accept: "application/json",
    ...extra,
  });

  const api = async (path, opts = {}) => {
    const res = await fetch(API_BASE + path, {
      ...opts,
      headers: { ...headers(), ...(opts.headers || {}) },
    });
    if (res.status === 401 || res.status === 403) {
      logout();
      throw new Error("Unauthorized");
    }
    if (!res.ok) {
      let msg;
      try { msg = (await res.json()).error || res.statusText; } catch { msg = res.statusText; }
      throw new Error(msg || "Request failed (" + res.status + ")");
    }
    if (res.status === 204) return null;
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? res.json() : res.text();
  };

  // --- Auth ---
  const showAuth = () => {
    $("auth").hidden = false;
    $("app").hidden = true;
  };
  const showApp = () => {
    $("auth").hidden = true;
    $("app").hidden = false;
  };
  const logout = () => {
    state.token = "";
    state.project = null;
    state.chat = null;
    localStorage.removeItem(TOKEN_KEY);
    showAuth();
  };

  $("connect").addEventListener("click", async () => {
    const t = $("token").value.trim();
    if (!t) return;
    state.token = t;
    try {
      // Probe with a list-projects call.
      await api("/projects");
      localStorage.setItem(TOKEN_KEY, t);
      $("auth-error").hidden = true;
      showApp();
      renderProjects();
    } catch (e) {
      state.token = "";
      const err = $("auth-error");
      err.textContent = e.message || "Connection failed";
      err.hidden = false;
    }
  });

  $("logout-btn").addEventListener("click", logout);
  $("back-btn").addEventListener("click", () => {
    if (state.view === "chat") openProject(state.project);
    else if (state.view === "project") renderProjects();
  });

  // --- Project list ---
  const renderProjects = async () => {
    state.view = "projects";
    state.project = null;
    state.chat = null;
    showOnly("view-projects");
    $("back-btn").hidden = true;
    try {
      const projects = await api("/projects");
      const list = $("project-list");
      list.innerHTML = "";
      $("projects-empty").hidden = projects.length > 0;
      for (const p of projects) {
        const li = document.createElement("li");
        li.className = "list-item";
        li.innerHTML =
          '<div><div class="item-title"></div><div class="item-sub">created ' +
          fmtDate(p.created_at) +
          "</div></div>";
        li.querySelector(".item-title").textContent = p.name;
        li.addEventListener("click", () => openProject(p));
        list.appendChild(li);
      }
    } catch (e) {
      toast(e.message, "error");
    }
  };

  $("new-project-btn").addEventListener("click", () => {
    $("new-project-form").hidden = false;
    $("new-project-name").focus();
  });
  $("new-project-cancel").addEventListener("click", () => {
    $("new-project-form").hidden = true;
    $("new-project-name").value = "";
  });
  $("new-project-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = $("new-project-name").value.trim();
    if (!name) return;
    try {
      await api("/projects", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name }),
      });
      $("new-project-form").hidden = true;
      $("new-project-name").value = "";
      renderProjects();
    } catch (err) {
      toast(err.message, "error");
    }
  });

  // --- Project detail ---
  const openProject = async (proj) => {
    state.view = "project";
    state.project = proj;
    state.chat = null;
    showOnly("view-project");
    $("back-btn").hidden = false;
    $("project-title").textContent = proj.name;
    activateTab("documents");
    await Promise.all([loadDocuments(proj.id), loadChats(proj.id)]);
  };

  const loadDocuments = async (projectId) => {
    try {
      const detail = await api("/projects/" + encodeURIComponent(projectId));
      const docs = detail.documents || [];
      const list = $("document-list");
      list.innerHTML = "";
      $("documents-empty").hidden = docs.length > 0;
      for (const d of docs) {
        const li = document.createElement("li");
        li.className = "list-item";
        const sizeKB = d.bytes ? Math.max(1, Math.round(d.bytes / 1024)) : 0;
        li.innerHTML =
          '<div><div class="item-title"></div><div class="item-sub"></div></div>' +
          '<div class="item-meta">' +
          (sizeKB ? sizeKB + " KB" : "") +
          "</div>";
        li.querySelector(".item-title").textContent = d.filename;
        li.querySelector(".item-sub").textContent =
          (d.content_type || "").replace(/^application\//, "") +
          (d.page_count ? " · " + d.page_count + " pages" : "");
        list.appendChild(li);
      }
      // Update state cache for chat-view "no docs" detection.
      state.project.documents = docs;
    } catch (e) {
      toast(e.message, "error");
    }
  };

  const loadChats = async (projectId) => {
    try {
      const chats = await api("/projects/" + encodeURIComponent(projectId) + "/chats");
      const list = $("chat-list");
      list.innerHTML = "";
      $("chats-empty").hidden = chats.length > 0;
      for (const c of chats) {
        const li = document.createElement("li");
        li.className = "list-item";
        const title = c.title || "(untitled chat)";
        li.innerHTML =
          '<div><div class="item-title"></div><div class="item-sub">' +
          fmtDate(c.created_at) +
          "</div></div>";
        li.querySelector(".item-title").textContent = title;
        li.addEventListener("click", () => openChat(c));
        list.appendChild(li);
      }
    } catch (e) {
      // Stream B (chat) may not be deployed yet — show graceful empty.
      $("chat-list").innerHTML = "";
      $("chats-empty").hidden = false;
    }
  };

  // --- Tabs (project view) ---
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => activateTab(tab.dataset.tab));
  });
  const activateTab = (name) => {
    document.querySelectorAll(".tab").forEach((t) => {
      t.classList.toggle("active", t.dataset.tab === name);
    });
    $("tab-documents").hidden = name !== "documents";
    $("tab-chats").hidden = name !== "chats";
  };

  // --- Document upload ---
  const uploadInput = $("upload-input");
  const uploadForm = $("upload-form");
  const uploadBtn = uploadForm.querySelector("button[type=submit]");
  uploadInput.addEventListener("change", () => {
    const has = uploadInput.files && uploadInput.files.length > 0;
    uploadBtn.disabled = !has;
    uploadInput.parentElement.classList.toggle("has-file", has);
    if (has) {
      uploadInput.parentElement.querySelector("span").textContent = uploadInput.files[0].name;
    }
  });
  uploadForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!uploadInput.files.length || !state.project) return;
    const file = uploadInput.files[0];
    const fd = new FormData();
    fd.append("file", file, file.name);
    showStatus("Uploading…", "ok");
    try {
      await api("/projects/" + encodeURIComponent(state.project.id) + "/documents", {
        method: "POST",
        body: fd,
      });
      showStatus("Uploaded", "ok");
      uploadInput.value = "";
      uploadBtn.disabled = true;
      uploadInput.parentElement.classList.remove("has-file");
      uploadInput.parentElement.querySelector("span").textContent = "Choose PDF or DOCX";
      await loadDocuments(state.project.id);
    } catch (err) {
      showStatus(err.message, "err");
    }
  });
  const showStatus = (msg, kind) => {
    const el = $("upload-status");
    el.textContent = msg;
    el.className = "status " + kind;
    el.hidden = false;
    if (kind === "ok") setTimeout(() => { el.hidden = true; }, 3000);
  };

  // --- New chat ---
  $("new-chat-btn").addEventListener("click", async () => {
    if (!state.project) return;
    try {
      const chat = await api("/projects/" + encodeURIComponent(state.project.id) + "/chats", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      openChat(chat);
    } catch (e) {
      toast(e.message, "error");
    }
  });

  // --- Chat view ---
  const openChat = async (chat) => {
    state.view = "chat";
    state.chat = chat;
    showOnly("view-chat");
    $("back-btn").hidden = false;
    $("chat-title").textContent = chat.title || "Chat";
    $("messages").innerHTML = "";
    try {
      const detail = await api("/chats/" + encodeURIComponent(chat.id));
      const msgs = detail.messages || [];
      for (const m of msgs) appendMessage(m.role, m.content);
      state.chat = detail;
    } catch (e) {
      toast("Could not load chat: " + e.message, "error");
    }
  };

  const appendMessage = (role, content) => {
    const m = document.createElement("div");
    m.className = "message " + (role === "user" ? "user" : "assistant");
    const tag = document.createElement("div");
    tag.className = "role-tag";
    tag.textContent = role;
    const body = document.createElement("div");
    body.className = "body";
    body.textContent = content;
    m.appendChild(tag);
    m.appendChild(body);
    const box = $("messages");
    box.appendChild(m);
    box.scrollTop = box.scrollHeight;
    return body;
  };

  $("composer").addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!state.chat) return;
    const input = $("composer-input");
    const text = input.value.trim();
    if (!text) return;
    input.value = "";
    appendMessage("user", text);
    const assistantBody = appendMessage("assistant", "");
    $("send-btn").disabled = true;
    try {
      const res = await fetch(API_BASE + "/chats/" + encodeURIComponent(state.chat.id) + "/messages", {
        method: "POST",
        headers: { ...headers(), "Content-Type": "application/json", "Accept": "text/event-stream" },
        body: JSON.stringify({ content: text }),
      });
      if (!res.ok) throw new Error("Send failed (" + res.status + ")");
      // SSE parsing: lines like "event: legal.message.delta\ndata: {...}\n\n".
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let idx;
        while ((idx = buf.indexOf("\n\n")) !== -1) {
          const block = buf.slice(0, idx);
          buf = buf.slice(idx + 2);
          const ev = parseSseBlock(block);
          if (ev?.event === "legal.message.delta" && ev.data?.content) {
            assistantBody.textContent += ev.data.content;
            $("messages").scrollTop = $("messages").scrollHeight;
          }
        }
      }
    } catch (err) {
      assistantBody.textContent = "[error] " + err.message;
    } finally {
      $("send-btn").disabled = false;
    }
  });

  const parseSseBlock = (block) => {
    let event = "message", data = "";
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) data += line.slice(5).trim();
    }
    let json = null;
    try { json = data ? JSON.parse(data) : null; } catch {}
    return { event, data: json };
  };

  // --- Export DOCX ---
  $("export-docx-btn").addEventListener("click", async () => {
    if (!state.chat) return;
    try {
      const res = await fetch(
        API_BASE + "/chats/" + encodeURIComponent(state.chat.id) + "/export.docx",
        { method: "POST", headers: headers() },
      );
      if (!res.ok) throw new Error("Export failed (" + res.status + ")");
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "chat-" + state.chat.id + ".docx";
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (e) {
      toast(e.message, "error");
    }
  });

  // --- View switching ---
  const showOnly = (id) => {
    document.querySelectorAll(".view").forEach((v) => { v.hidden = v.id !== id; });
  };

  // --- Date formatter ---
  const fmtDate = (epoch) => {
    if (!epoch) return "";
    // Server returns seconds; some streams return milliseconds. Normalize.
    const ms = epoch < 1e12 ? epoch * 1000 : epoch;
    return new Date(ms).toLocaleString();
  };

  // --- Boot ---
  if (state.token) {
    showApp();
    renderProjects();
  } else {
    showAuth();
  }
})();
