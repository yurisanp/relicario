// Interface do Relicário. Telas do protótipo (docs/design/prototipo.html) com dados reais do
// daemon, pelos comandos do Tauri: `api` (veja crates/relicario-daemon/src/api.rs) e `daemon`.
(function () {
  "use strict";
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => Array.from(el.querySelectorAll(s));
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const t = window.I18N.t;

  // ---------- ponte com o Rust ----------
  const tauri = window.__TAURI__ && window.__TAURI__.core;
  const invoke = (cmd, args) => tauri ? tauri.invoke(cmd, args) : Promise.reject(t("err.notauri"));
  const api = (call) => invoke("api", { call });
  const daemon = (req) => invoke("daemon", { req });

  // ---------- estado ----------
  const S = {
    ov: null,            // overview
    games: new Map(),    // detalhe por jogo
    snaps: new Map(),    // detalhe por snapshot
    art: new Map(),      // "id:tipo" -> {url, author, source}
    filter: "todos", q: "", libView: localStorage.getItem("libView") || "grade",
    gameTab: "linha", sel: null, picker: false, pickKind: "capa",
    pick: null, choice: "keep", busy: false, pairing: null,
  };
  const store = { get: (k) => { try { return localStorage.getItem(k); } catch { return null; } }, set: (k, v) => { try { localStorage.setItem(k, v); } catch { /* sem armazenamento */ } } };

  // ---------- formatação ----------
  const lang = () => window.I18N.lang();
  const fmtSize = (n) => {
    if (n == null) return "—";
    const u = ["B", "KB", "MB", "GB", "TB"]; let i = 0; let v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v.toLocaleString(lang(), { maximumFractionDigits: v < 10 && i > 0 ? 1 : 0 })} ${u[i]}`;
  };
  const fmtWhen = (ms) => {
    if (!ms) return t("never");
    const d = new Date(ms), now = new Date();
    const hm = d.toLocaleTimeString(lang(), { hour: "2-digit", minute: "2-digit" });
    const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const diff = Math.round((day(now) - day(d)) / 86400000);
    if (diff === 0) return t("today", { hm });
    if (diff === 1) return t("yesterday", { hm });
    return d.toLocaleDateString(lang(), { day: "2-digit", month: "2-digit", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" }) + ", " + hm;
  };
  const fmtAgo = (ms) => {
    if (!ms) return t("never");
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 90) return t("ago.now");
    if (s < 3600) return t("ago.min", { n: Math.round(s / 60) });
    if (s < 86400) return t("ago.h", { n: Math.round(s / 3600) });
    return t("ago.d", { n: Math.round(s / 86400) });
  };
  const fmtPlay = (ms) => {
    if (!ms) return "—";
    const m = Math.round(ms / 60000), h = Math.floor(m / 60);
    return h ? `${h}h${String(m % 60).padStart(2, "0")}` : `${m} min`;
  };
  const hue = (s) => { let h = 0; for (const c of String(s)) h = (h * 31 + c.charCodeAt(0)) >>> 0; return h % 360; };
  const mcolor = (id) => `hsl(${hue(id)} 70% 70%)`;
  const mtag = (id, name) => id ? `<span class="mtag" style="color:${mcolor(id)}">${esc(name || id)}</span>` : "";
  const initials = (s) => String(s).split(/[\s:'-]+/).filter(Boolean).slice(0, 2).map(w => w[0]).join("").toUpperCase() || "?";

  // ---------- avisos ----------
  let toastTimer;
  function toast(msg, bad) {
    const el = $("#toast");
    el.textContent = msg;
    el.style.background = bad ? "var(--bad)" : "";
    el.classList.add("show");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove("show"), bad ? 6000 : 3500);
  }
  const fail = (e) => toast(String(e), true);

  // Botão ocupado enquanto a promessa roda.
  async function busy(btn, p) {
    const old = btn ? btn.innerHTML : null;
    if (btn) { btn.disabled = true; btn.innerHTML = `<span class="spinner" aria-hidden="true"></span> ${old}`; }
    try { return await p; } finally { if (btn && btn.isConnected) { btn.disabled = false; btn.innerHTML = old; } }
  }

  // ---------- modal ----------
  let modalReturn = null;
  function modal(html, onMount) {
    modalReturn = document.activeElement;
    $("#modal").innerHTML = `<div class="modal-bg" data-close-bg><div class="modal" role="dialog" aria-modal="true">${html}</div></div>`;
    const m = $("#modal .modal");
    const first = m.querySelector("input, select, button");
    if (first) first.focus();
    if (onMount) onMount(m);
    return m;
  }
  function closeModal() {
    $("#modal").innerHTML = "";
    if (modalReturn && modalReturn.isConnected) modalReturn.focus();
  }

  // ---------- arte ----------
  async function art(id, kind) {
    const k = `${id}:${kind}`;
    if (S.art.has(k)) return S.art.get(k);
    const p = api({ call: "art", game: id, kind }).catch(() => null);
    S.art.set(k, p);
    const v = await p;
    S.art.set(k, v);
    return v;
  }
  // Preenche os elementos com data-art depois de desenhar a tela.
  function hydrateArt(root) {
    $$("[data-art]", root).forEach(async (el) => {
      const [id, kind] = el.dataset.art.split("|");
      const a = await art(id, kind);
      if (!a || !el.isConnected) return;
      if (!el.querySelector("img")) el.insertAdjacentHTML("afterbegin", `<img alt="" src="${esc(a.url)}">`);
      el.classList.add("has-img");
      const cr = el.querySelector(".cr");
      if (cr) cr.textContent = a.author ? t("art.credit", { a: a.author }) : a.source === "gerada" ? "" : t(`art.src.${a.source}`) || a.source;
    });
  }
  const poster = (g, mini) => `<span class="poster${mini ? " mini" : ""} fallback" data-art="${esc(g.id)}|capa" style="background:linear-gradient(160deg,hsl(${hue(g.id)} 35% 34%),hsl(${(hue(g.id) + 40) % 360} 40% 18%))" role="img" aria-label="${esc(t("art.cover", { g: g.title }))}"><span><span class="mono-l">${esc(initials(g.title))}</span>${mini ? "" : `<small>${esc(g.title)}</small>`}</span>${mini ? "" : `<span class="cr"></span>`}</span>`;

  // ---------- estados ----------
  const pill = (state, g) => {
    const m = {
      conflito: ["warn", t("st.conflict")], suspeito: ["bad", t("st.suspect")], pendente: ["sync", t("st.pending")],
      aberto_aqui: ["brass", t("st.open_here")], so_historico: ["", t("st.history")], em_dia: ["ok", t("st.ok")],
      sem_save: ["plain", t("st.nosave")], nao_instalado: ["plain", t("st.notinstalled")],
    }[state] || ["", state];
    let extra = "";
    if (g && g.running_elsewhere && g.running_elsewhere.length) extra = ` <span class="pill brass">${esc(t("st.open_at", { m: g.running_elsewhere.join(", ") }))}</span>`;
    return `<span class="pill ${m[0]}">${esc(m[1])}</span>${extra}`;
  };
  const attention = (g) => ["conflito", "suspeito", "pendente"].includes(g.state);

  // ---------- rotas ----------
  const R = {};
  function route() {
    const h = decodeURIComponent(location.hash.slice(1) || "biblioteca");
    const [name, arg] = h.split("/");
    return { name, arg };
  }
  async function render() {
    const { name, arg } = route();
    $$("#appnav a").forEach(a => a.toggleAttribute("aria-current", a.dataset.r === name || (name === "jogo" && a.dataset.r === "biblioteca") || (name === "conflito" && a.dataset.r === "conflitos")));
    $$("#appnav a[aria-current]").forEach(a => a.setAttribute("aria-current", "page"));
    const fn = R[name] || R.biblioteca;
    const view = $("#view");
    try {
      const html = await fn(arg);
      if (route().name !== name || route().arg !== arg) return; // trocou de tela no meio
      view.innerHTML = html;
      hydrateArt(view);
      if (R[name + "_mounted"]) R[name + "_mounted"](arg);
    } catch (e) {
      view.innerHTML = `<div class="panel"><div class="panel-b err">${esc(e)}</div></div>`;
    }
  }

  // ---------- barra lateral e estado ----------
  async function refreshOverview() {
    try {
      S.ov = await api({ call: "overview" });
    } catch (e) {
      $("#syncstate").innerHTML = `<span class="dot warn"></span><span>${esc(e)}</span>`;
      return;
    }
    const ov = S.ov;
    const c = $("#ccount");
    c.hidden = !ov.conflicts; c.textContent = ov.conflicts;
    const now = Date.now();
    $("#machines").innerHTML = `<div class="label">${esc(t("side.machines"))}</div>` + ov.machines.map(m => {
      const recent = m.me || now - m.updated_at < 15 * 60000;
      const sub = m.running.length ? t("side.playing", { g: m.running.join(", ") }) : m.me ? t("side.this") : t("side.seen", { w: fmtAgo(m.updated_at) });
      return `<div class="mchip"><span class="dot ${m.running.length ? "sync" : recent ? "ok" : "off"}"></span><b style="color:${mcolor(m.id)}">${esc(m.name)}${m.me ? ` <span class="mono" style="color:var(--faint)">${esc(t("side.me"))}</span>` : ""}</b><span>${esc(sub)}</span></div>`;
    }).join("");
    let st;
    if (ov.cloud_busy) st = `<span class="dot sync"></span><span>${esc(t("top.uploading"))}</span>`;
    else if (ov.running.length) st = `<span class="dot sync"></span><span>${esc(t("top.playing", { g: ov.running.join(", ") }))}</span>`;
    else if (ov.conflicts) st = `<span class="dot warn"></span><span>${esc(t("top.conflicts", { n: ov.conflicts }))}</span>`;
    else st = `<span class="dot ok"></span><span>${esc(t("top.ok"))}</span>`;
    $("#syncstate").innerHTML = st + ` <button class="btn ghost" data-act="syncall" title="${esc(t("act.syncnow"))}">${esc(t("act.syncnow"))}</button>`;
  }

  // ---------- Biblioteca ----------
  R.biblioteca = async () => {
    if (!S.ov) await refreshOverview();
    const ov = S.ov;
    if (!ov) return `<div class="empty">${esc(t("err.nodaemon"))}</div>`;
    const games = ov.games;
    const match = (g) => {
      if (S.q && !(g.title + " " + g.id + " " + (g.store || "")).toLowerCase().includes(S.q.toLowerCase())) return false;
      if (S.filter === "atencao") return attention(g);
      if (S.filter === "aqui") return g.installed;
      if (S.filter === "historico") return g.state === "so_historico" || g.history_only;
      return true;
    };
    const counts = { todos: games.length, atencao: games.filter(attention).length, aqui: games.filter(g => g.installed).length, historico: games.filter(g => g.history_only).length };
    const list = games.filter(match);
    const chip = (k) => `<button class="chip" data-filter="${k}" aria-pressed="${S.filter === k}">${esc(t("lib.f." + k))}<span class="n">${counts[k]}</span></button>`;
    const conflict = games.find(g => g.state === "conflito");
    const snaps = games.reduce((a, g) => a + g.snapshots, 0);
    const size = games.reduce((a, g) => a + (g.size || 0), 0);
    const lastAt = Math.max(0, ...games.map(g => g.last_at || 0));
    const other = ov.machines.filter(m => !m.me).sort((a, b) => b.updated_at - a.updated_at)[0];
    const sub = (g) => `${g.last_machine ? esc(g.last_machine) + " · " : ""}${esc(fmtWhen(g.last_at))}`;
    const body = !games.length ? `<div class="empty"><b>${esc(t("lib.empty"))}</b><span>${esc(t("lib.empty.hint"))}</span><button class="btn primary" data-act="rescan">${esc(t("act.rescan"))}</button></div>`
      : S.libView === "grade" ? `<div class="ggrid">${list.map(g => `<button class="gcard" data-go="jogo/${esc(g.id)}">${poster(g)}<b>${esc(g.title)}</b><span class="row">${pill(g.state, g)}</span><span class="small sub">${sub(g)}</span></button>`).join("") || `<span class="sub">${esc(t("lib.nomatch"))}</span>`}</div>`
      : `<div class="ghead eyebrow"><span></span><span>${esc(t("lib.col.game"))}</span><span>${esc(t("lib.col.state"))}</span><span>${esc(t("lib.col.last"))}</span><span>${esc(t("lib.col.size"))}</span><span></span></div>
        <div class="glist">${list.map(g => `<button class="grow" data-go="jogo/${esc(g.id)}">${poster(g, true)}
          <span style="min-width:0"><span class="name">${esc(g.title)}</span><span class="src">${esc([g.store, g.launcher].filter(Boolean).join(" · ") || g.id)}</span></span>
          <span>${pill(g.state, g)}</span><span class="last">${sub(g)}</span><span class="size num">${esc(fmtSize(g.size))}</span><span class="chev">›</span></button>`).join("") || `<div class="panel-b sub">${esc(t("lib.nomatch"))}</div>`}</div>`;
    return `
    ${conflict ? `<div class="banner"><span class="ic" aria-hidden="true"><svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 3v6a4 4 0 0 0 4 4M14 3v6a4 4 0 0 1-4 4v4"/></svg></span>
      <div class="txt"><b>${esc(t("lib.banner", { g: conflict.title }))}</b><span>${esc(t("lib.banner.sub"))}</span></div>
      <a class="btn primary" href="#conflito/${esc(conflict.id)}">${esc(t("act.resolve"))}</a></div>` : ""}
    <div class="pagehead"><div><h1>${esc(t("nav.library"))}</h1><p>${esc(t("lib.sub", { n: games.length, m: ov.machines.length }))}</p></div>
      <div class="actions"><button class="btn" data-act="rescan">${esc(t("act.rescan"))}</button><button class="btn primary" data-act="syncall">${esc(t("act.syncnow"))}</button></div></div>
    <div class="stats">
      <div class="stat"><span class="v num">${snaps}</span><span class="k">${esc(t("lib.stat.snaps"))}</span></div>
      <div class="stat"><span class="v num">${esc(fmtSize(size))}</span><span class="k">${esc(t("lib.stat.size"))}</span></div>
      <div class="stat"><span class="v num">${esc(fmtAgo(lastAt))}</span><span class="k">${esc(t("lib.stat.last"))}</span></div>
      <div class="stat"><span class="v num">${other ? esc(fmtAgo(other.updated_at)) : "—"}</span><span class="k">${esc(other ? t("lib.stat.other", { m: other.name }) : t("lib.stat.alone"))}</span></div>
    </div>
    <div class="panel">
      <div class="panel-h"><div class="chips" role="group" aria-label="${esc(t("lib.filter"))}">${chip("todos")}${chip("atencao")}${chip("aqui")}${chip("historico")}</div>
        <div class="seg" role="group" aria-label="${esc(t("lib.view"))}"><button data-libview="grade" aria-pressed="${S.libView === "grade"}">${esc(t("lib.grid"))}</button><button data-libview="lista" aria-pressed="${S.libView === "lista"}">${esc(t("lib.list"))}</button></div></div>
      ${body}
    </div>`;
  };

  // ---------- Jogo ----------
  async function gameDetail(id, fresh) {
    if (!fresh && S.games.has(id)) return S.games.get(id);
    const d = await api({ call: "game", id });
    S.games.set(id, d);
    return d;
  }
  async function snapDetail(game, sid) {
    const k = game + ":" + sid;
    if (S.snaps.has(k)) return S.snaps.get(k);
    const d = await api({ call: "snapshot", game, snapshot: sid });
    S.snaps.set(k, d);
    return d;
  }

  // Árvore: uma coluna por máquina, setas até os pais.
  function graph(snaps) {
    const rowH = 72, lanes = [...new Set(snaps.map(s => s.machine))];
    const laneX = (m) => 18 + lanes.indexOf(m) * 26;
    const idx = new Map(snaps.map((s, i) => [s.id, i]));
    const cy = (i) => i * rowH + rowH / 2;
    let paths = "", nodes = "";
    snaps.forEach((s, i) => s.parents.forEach((p) => {
      const j = idx.get(p); if (j === undefined) return;
      const x1 = laneX(snaps[j].machine), y1 = cy(j), x2 = laneX(s.machine), y2 = cy(i);
      paths += x1 === x2 ? `<path d="M${x1} ${y1} L${x2} ${y2}" stroke="var(--faint)" stroke-width="2" fill="none"/>`
        : `<path d="M${x1} ${y1} C${x1} ${y1 - 34}, ${x2} ${y2 + 34}, ${x2} ${y2}" stroke="var(--faint)" stroke-width="2" fill="none"/>`;
    }));
    snaps.forEach((s, i) => {
      const x = laneX(s.machine), y = cy(i), c = mcolor(s.machine);
      nodes += `<circle cx="${x}" cy="${y}" r="${s.pinned ? 7 : 6}" fill="var(--surface)" stroke="${c}" stroke-width="2.5"${s.forgotten ? ' stroke-dasharray="3 2"' : ""}/>`;
      if (s.pinned) nodes += `<circle cx="${x}" cy="${y}" r="2.6" fill="var(--brass)"/>`;
      if (s.head_of.length) nodes += `<circle cx="${x}" cy="${y}" r="10" fill="none" stroke="${c}" stroke-width="1" stroke-dasharray="2 2"/>`;
    });
    const w = Math.max(92, 36 + lanes.length * 26);
    return { svg: `<svg class="graph" width="${w}" height="${snaps.length * rowH}" viewBox="0 0 ${w} ${snaps.length * rowH}" aria-hidden="true">${paths}${nodes}</svg>`, w, lanes };
  }

  const reasonText = (r) => t("reason." + r);
  const fieldName = (f) => t("field." + f) || f;

  function metaKv(meta) {
    if (!meta) return "";
    return meta.fields.map(([k, v]) => `<dt>${esc(fieldName(k))}</dt><dd>${esc(v)}</dd>`).join("");
  }

  function snapPanel(g, s, det) {
    const meta = det && det.meta;
    const thumb = meta && meta.thumbnail ? `<img class="thumb" alt="${esc(t("snap.thumb"))}" src="${esc(meta.thumbnail)}">` : "";
    return `<div class="panel" id="snappanel">
      <div class="panel-h"><h3>${esc(t("snap.title"))} <span class="mono">${esc(s.short)}</span></h3>${mtag(s.machine, s.machine_name)}</div>
      <div class="panel-b" style="display:grid;gap:14px">
        ${thumb}
        ${s.pinned ? `<span class="pill brass plain">📌 ${esc(s.pinned)}</span>` : ""}
        ${s.suspicions.length ? `<span class="pill bad">${esc(t("snap.suspect", { r: s.suspicions.join("; ") }))}</span>` : ""}
        ${s.forgotten ? `<span class="pill plain">${esc(t("snap.forgotten"))}</span>` : ""}
        <dl class="kv">
          <dt>${esc(t("snap.created"))}</dt><dd>${esc(fmtWhen(s.created_at))}</dd>
          <dt>${esc(t("snap.reason"))}</dt><dd>${esc(reasonText(s.reason))}</dd>
          ${metaKv(meta)}
          <dt>${esc(t("snap.parent"))}</dt><dd class="mono">${s.parents.length ? s.parents.map(p => esc(p.slice(0, 10))).join(", ") : esc(t("snap.first"))}</dd>
          <dt>${esc(t("snap.size"))}</dt><dd>${esc(fmtSize(s.size))} · ${esc(t("snap.files", { n: s.files }))}</dd>
          ${s.head_of.length ? `<dt>${esc(t("snap.current"))}</dt><dd>${esc(s.head_of.join(", "))}</dd>` : ""}
          ${meta ? `<dt>${esc(t("snap.readby"))}</dt><dd>${esc(t("snap.reader", { r: meta.reader, f: meta.source.split("/").pop() }))}</dd>` : ""}
        </dl>
        ${det ? "" : `<span class="sub small"><span class="spinner" aria-hidden="true"></span> ${esc(t("loading"))}</span>`}
        <div class="actions">
          <button class="btn primary" data-act="restore" data-sid="${s.id}" ${s.forgotten || !g.installed || s.local_head ? "disabled" : ""}>${esc(s.local_head ? t("snap.isnow") : t("act.restore"))}</button>
          <button class="btn" data-act="pin" data-sid="${s.id}">${esc(s.pinned ? t("act.rename") : t("act.pin"))}</button>
          ${s.pinned ? `<button class="btn ghost" data-act="unpin" data-sid="${s.id}">${esc(t("act.unpin"))}</button>` : ""}
          <button class="btn ghost" data-act="zip" data-sid="${s.id}" ${s.forgotten ? "disabled" : ""}>${esc(t("act.zip"))}</button>
        </div>
        <p class="small sub" style="margin:0">${esc(t("snap.safety"))}</p>
      </div></div>`;
  }

  R.jogo = async (id) => {
    const g = await gameDetail(id, true);
    const snaps = g.snapshots;
    if (!S.sel || !snaps.some(s => s.id === S.sel)) S.sel = (snaps.find(s => s.local_head) || snaps[0] || {}).id;
    const ovg = S.ov && S.ov.games.find(x => x.id === id);
    const state = ovg ? ovg.state : (g.plan && g.plan.action.kind === "conflito" ? "conflito" : "em_dia");
    const conflict = g.plan && g.plan.action.kind === "conflito";
    const tabs = [["linha", t("game.tab.timeline")], ["arquivos", t("game.tab.files")], ["regras", t("game.tab.rules")]];
    let content;
    if (S.gameTab === "linha") {
      if (!snaps.length) content = `<div class="panel"><div class="empty">${esc(t("game.nosnaps"))}</div></div>`;
      else {
        const gr = graph(snaps);
        const s = snaps.find(x => x.id === S.sel);
        content = `<div class="split">
          <div class="panel">
            <div class="lanes">${gr.lanes.map(m => { const sn = snaps.find(x => x.machine === m); return `<span class="lane-tag" style="color:${mcolor(m)};width:auto;margin-right:6px">${esc(sn.machine_name)}</span>`; }).join("")}<span style="width:auto;margin-left:auto;color:var(--faint)" class="small">${esc(t("game.tree.hint"))}</span></div>
            <div class="tree" role="list">${gr.svg}
              ${snaps.map(x => `<button class="snap" role="listitem" data-snap="${x.id}" aria-pressed="${x.id === S.sel}" style="grid-template-columns:${gr.w}px minmax(0,1fr) auto">
                <span></span>
                <span style="min-width:0"><span class="t1">${esc(x.pinned || reasonText(x.reason))}${x.local_head ? ` <span class="pill ok">${esc(t("snap.now"))}</span>` : ""}${x.suspicions.length ? ` <span class="pill bad">${esc(t("st.suspect"))}</span>` : ""}</span>
                <span class="t2">${esc(x.machine_name)} · ${esc(fmtSize(x.size))} · <span class="mono">${esc(x.short)}</span></span></span>
                <span class="when">${esc(fmtWhen(x.created_at))}</span></button>`).join("")}
            </div>
          </div>
          ${s ? snapPanel(g, s, S.snaps.get(id + ":" + s.id)) : ""}
        </div>`;
      }
    } else if (S.gameTab === "arquivos") {
      const s = snaps.find(x => x.id === S.sel);
      const det = s ? await snapDetail(id, s.id).catch(() => null) : null;
      content = `<div class="panel"><div class="panel-h"><h3>${esc(t("game.files.title"))}</h3></div>
        <div class="files">${g.sources.map(r => `<div class="frow"><span class="p">${esc(r.root)} → ${esc(r.path)}</span><span class="num sub">${esc([...r.include.map(x => "+" + x), ...r.exclude.map(x => "−" + x)].join(" "))}</span><span class="pill ok">${esc(t("game.files.in"))}</span></div>`).join("") || `<div class="frow"><span class="p sub">${esc(t("game.files.nosource"))}</span></div>`}
        ${g.excludes.map(x => `<div class="frow ign"><span class="p">${esc(x)}</span><span></span><span class="pill">${esc(t("game.files.ignored"))}</span></div>`).join("")}</div>
        ${det ? `<div class="panel-h" style="border-top:1px solid var(--line)"><h3>${esc(t("game.files.insnap", { s: s.short }))}</h3></div>
        <div class="files">${det.files.map(f => `<div class="frow"><span class="p">${esc(f.root)}/${esc(f.path)}</span><span class="num sub">${esc(fmtSize(f.size))}</span><span class="sub small">${esc(fmtWhen(f.mtime))}</span></div>`).join("")}</div>` : ""}
      </div>`;
    } else {
      const mode = g.ignored ? "ignorar" : g.history_only ? "historico" : "completo";
      content = `<div class="panel">
        <div class="setrow"><div><b>${esc(t("game.rules.mode"))}</b><p>${esc(t("game.rules.mode.hint"))}${g.steam_cloud ? " " + esc(t("game.rules.steamcloud")) : ""}</p></div>
          <div class="seg" role="group" aria-label="${esc(t("game.rules.mode"))}"><button data-mode="completo" aria-pressed="${mode === "completo"}">${esc(t("mode.completo"))}</button><button data-mode="historico" aria-pressed="${mode === "historico"}">${esc(t("mode.historico"))}</button><button data-mode="ignorar" aria-pressed="${mode === "ignorar"}">${esc(t("mode.ignorar"))}</button></div></div>
        <div class="setrow"><div><b>${esc(t("game.rules.launcher"))}</b><p>${esc(t("game.rules.launcher.hint"))} <span class="mono">relicario run --game ${esc(g.id)} -- %command%</span></p></div></div>
        <div class="setrow"><div><b>${esc(t("game.rules.exe"))}</b><p>${esc(g.executables.join(", ") || t("game.rules.exe.none"))}</p></div></div>
        <div class="setrow"><div><b>${esc(t("game.rules.playtime"))}</b><p>${esc(g.playtime.map(p => `${p.machine}: ${fmtPlay(p.ms)}`).join(" · ") || "—")}</p></div></div>
      </div>`;
    }
    const total = g.playtime.reduce((a, p) => a + p.ms, 0);
    return `
      <div><a href="#biblioteca" class="btn ghost" style="padding-left:0">‹ ${esc(t("nav.library"))}</a></div>
      <div class="hero" data-art="${esc(id)}|banner" style="background:linear-gradient(120deg,hsl(${hue(id)} 35% 30%),hsl(${(hue(id) + 40) % 360} 40% 14%))" role="img" aria-label="${esc(t("art.banner", { g: g.title }))}"><span class="logo-art">${esc(g.title)}</span><button class="btn small chg" data-act="picker">${esc(t("art.change"))}</button><span class="cr"></span></div>
      ${S.picker ? `<div id="pickerbox">${await picker(g)}</div>` : ""}
      <div class="gamehead"><span style="width:84px;flex:none">${poster(g)}</span>
        <div class="info"><h1>${esc(g.title)}</h1>
          <div class="meta">${pill(state, ovg)}<span>${esc([g.store, g.store_id].filter(Boolean).join(" · ") || g.id)}</span><span class="num">${esc(t("game.snaps", { n: snaps.length }))}</span>${total ? `<span>${esc(t("game.played", { t: fmtPlay(total) }))}</span>` : ""}</div></div>
        <div class="actions">${conflict ? `<a class="btn primary" href="#conflito/${esc(id)}">${esc(t("act.resolve"))}</a>` : ""}<button class="btn" data-act="syncgame" ${g.installed ? "" : "disabled"}>${esc(t("act.syncgame"))}</button></div>
      </div>
      ${g.running_here ? `<div class="banner" style="background:var(--brass-soft)"><div class="txt"><b>${esc(t("game.open"))}</b><span>${esc(t("game.open.sub"))}</span></div></div>` : ""}
      <div class="tabs" role="tablist">${tabs.map(x => `<button class="tab" role="tab" data-tab="${x[0]}" aria-selected="${S.gameTab === x[0]}">${esc(x[1])}</button>`).join("")}</div>
      ${content}`;
  };
  R.jogo_mounted = async (id) => {
    if (S.gameTab !== "linha" || !S.sel) return;
    loadSnap(id, S.sel);
  };
  async function loadSnap(id, sid) {
    const g = S.games.get(id); if (!g) return;
    const s = g.snapshots.find(x => x.id === sid); if (!s) return;
    const det = await snapDetail(id, sid).catch(() => ({ files: [], meta: null }));
    const el = $("#snappanel");
    if (el && S.sel === sid) el.outerHTML = snapPanel(g, s, det);
  }

  async function picker(g) {
    const kinds = ["capa", "banner", "logo", "icone"];
    let ch;
    try { ch = await api({ call: "art_choices", game: g.id, kind: S.pickKind }); } catch (e) { ch = { has_key: true, choices: [], error: String(e) }; }
    const opts = ch.choices.map((c, i) => `<button class="pick-opt" data-choice="${i}"><span class="poster" style="aspect-ratio:${S.pickKind === "banner" ? "1920/620" : S.pickKind === "capa" ? "2/3" : "1"}"><img alt="" src="${esc(c.thumb)}" loading="lazy"></span><span>${esc(c.author ? t("art.by", { a: c.author }) : "SteamGridDB")}</span></button>`).join("");
    S.choices = ch.choices;
    return `<div class="panel"><div class="panel-h"><h3>${esc(t("art.picker", { g: g.title }))}</h3><div class="seg" role="group">${kinds.map(k => `<button data-pickkind="${k}" aria-pressed="${S.pickKind === k}">${esc(t("art.kind." + k))}</button>`).join("")}</div></div>
      <div class="panel-b" style="display:grid;gap:14px">
        ${!ch.has_key ? `<span class="small sub">${esc(t("art.nokey"))} <a href="#ajustes" style="color:var(--brass)">${esc(t("nav.settings"))}</a></span>` : ch.error ? `<span class="small err">${esc(ch.error)}</span>` : `<span class="small sub">${esc(t("art.count", { n: ch.choices.length }))}</span>`}
        <div class="picker-grid">${opts}<button class="pick-add" data-act="artfile">${esc(t("art.upload"))}</button></div>
        <div class="actions"><button class="btn primary" data-act="picker">${esc(t("act.done"))}</button><button class="btn ghost" data-act="artreset">${esc(t("art.reset"))}</button></div>
      </div></div>`;
  }

  // ---------- Conflitos ----------
  R.conflitos = async () => {
    await refreshOverview();
    const list = S.ov.games.filter(g => g.state === "conflito");
    if (!list.length) return `<div class="pagehead"><div><h1>${esc(t("nav.conflicts"))}</h1><p>${esc(t("conf.none"))}</p></div></div>
      <div class="panel"><div class="panel-b sub">${esc(t("conf.none.sub"))}</div></div>`;
    return `<div class="pagehead"><div><h1>${esc(t("nav.conflicts"))}</h1><p>${esc(t("conf.count", { n: list.length }))}</p></div></div>
      <div class="panel"><div class="glist">${list.map(g => `<button class="grow" data-go="conflito/${esc(g.id)}" style="grid-template-columns:44px minmax(0,1fr) auto">${poster(g, true)}<span><span class="name">${esc(g.title)}</span><span class="src">${esc(g.status ? g.status.outcome.text : "")}</span></span><span class="btn primary">${esc(t("act.resolve"))}</span></button>`).join("")}</div></div>`;
  };

  // Ancestral comum mais novo das pontas.
  function commonBase(snaps, tips) {
    const by = new Map(snaps.map(s => [s.id, s]));
    const anc = (id) => { const seen = new Set(); const st = [id]; while (st.length) { const x = st.pop(); if (seen.has(x)) continue; seen.add(x); const s = by.get(x); if (s) st.push(...s.parents); } return seen; };
    const sets = tips.map(anc);
    const common = [...sets[0]].filter(x => !tips.includes(x) && sets.every(s => s.has(x)) && by.has(x));
    return common.map(x => by.get(x)).sort((a, b) => b.created_at - a.created_at)[0];
  }

  R.conflito = async (id) => {
    const g = await gameDetail(id, true);
    const a = g.plan && g.plan.action;
    if (!a || a.kind !== "conflito") {
      return `<div><a href="#jogo/${esc(id)}" class="btn ghost" style="padding-left:0">‹ ${esc(g.title)}</a></div>
        <div class="panel"><div class="panel-b" style="display:grid;gap:8px"><b>${esc(t("conf.solved", { g: g.title }))}</b><span class="sub">${esc(t("conf.solved.sub"))}</span><div><a class="btn" href="#jogo/${esc(id)}">${esc(t("conf.timeline"))}</a></div></div></div>`;
    }
    const tips = a.tips;
    const dets = await Promise.all(tips.map(tp => snapDetail(id, tp.snapshot).catch(() => null)));
    const snap = (sid) => g.snapshots.find(s => s.id === sid) || {};
    const base = commonBase(g.snapshots, tips.map(x => x.snapshot));
    if (!S.pick || !tips.some(x => x.snapshot === S.pick)) S.pick = null;
    const side = (tp, det) => {
      const s = snap(tp.snapshot), meta = det && det.meta;
      return `<button class="vside" data-pick="${tp.snapshot}" aria-pressed="${S.pick === tp.snapshot}">
        <span class="h"><b style="color:${mcolor(tp.machine)}">${esc(tp.name)}</b>${tp.local ? `<span class="pill plain">${esc(t("conf.thismachine"))}</span>` : ""}</span>
        ${meta && meta.thumbnail ? `<img class="thumb" alt="" src="${esc(meta.thumbnail)}">` : ""}
        <dl class="kv">
          <dt>${esc(t("conf.played"))}</dt><dd>${esc(fmtWhen(s.created_at))}</dd>
          ${metaKv(meta)}
          <dt>${esc(t("snap.size"))}</dt><dd class="num">${esc(fmtSize(s.size))} · ${esc(t("snap.files", { n: s.files }))}</dd>
          <dt>${esc(t("snap.id"))}</dt><dd class="mono">${esc(s.short)}</dd>
        </dl></button>`;
    };
    const opts = [
      ["keep", t("conf.opt.keep"), t("conf.opt.keep.sub")],
      ["both", t("conf.opt.both"), t("conf.opt.both.sub")],
      ["later", t("conf.opt.later"), t("conf.opt.later.sub")],
    ];
    const chosen = tips.find(x => x.snapshot === S.pick);
    const msg = S.choice === "later" ? t("conf.msg.later") : !chosen ? t("conf.msg.pick") : S.choice === "both" ? t("conf.msg.both", { m: chosen.name }) : t("conf.msg.keep", { m: chosen.name });
    return `
      <div><a href="#jogo/${esc(id)}" class="btn ghost" style="padding-left:0">‹ ${esc(g.title)}</a></div>
      <div class="pagehead"><div><span class="eyebrow">${esc(t("conf.eyebrow", { g: g.title }))}</span><h1>${esc(t("conf.title"))}</h1><p>${esc(t("conf.sub"))}</p></div></div>
      ${base ? `<div class="panel"><div class="panel-b ancestry"><span class="small sub">${esc(t("conf.base"))}</span><span class="mono">${esc(base.short)} · ${esc(fmtWhen(base.created_at))} · ${esc(base.machine_name)}</span></div></div>` : ""}
      <div class="vs">${tips.map((tp, i) => side(tp, dets[i])).join(`<div class="vmid" aria-hidden="true">${esc(t("conf.or"))}</div>`)}</div>
      <div class="opts" role="group" aria-label="${esc(t("conf.what"))}" style="grid-template-columns:repeat(3,minmax(0,1fr))">${opts.map(o => `<button class="opt" data-choice-opt="${o[0]}" aria-pressed="${S.choice === o[0]}"><b>${esc(o[1])}</b><span>${esc(o[2])}</span></button>`).join("")}</div>
      <div class="confirm"><span>${esc(msg)}</span><button class="btn primary" data-act="resolve" ${S.choice !== "later" && !chosen ? "disabled" : ""}>${esc(S.choice === "later" ? t("act.later") : t("act.confirm"))}</button></div>`;
  };

  // ---------- Atividade ----------
  const kindIcon = { guardado: "●", restaurado: "↺", conflito: "⑂", suspeito: "⚠", adiado: "…", aberto: "▶", fechado: "■", nuvem: "☁", par: "⇄", pareamento: "⚭", resolvido: "✓", fixado: "📌", erro: "✕", so_historico: "◌" };
  R.atividade = async () => {
    const list = await api({ call: "activity", limit: 300 });
    const byDay = [];
    for (const e of list) {
      const d = new Date(e.at).toLocaleDateString(lang(), { weekday: "long", day: "2-digit", month: "long" });
      if (!byDay.length || byDay[byDay.length - 1][0] !== d) byDay.push([d, []]);
      byDay[byDay.length - 1][1].push(e);
    }
    return `<div class="pagehead"><div><h1>${esc(t("nav.activity"))}</h1><p>${esc(t("act.sub"))}</p></div><div class="actions"><button class="btn" data-act="prune">${esc(t("act.prune"))}</button></div></div>
      ${byDay.length ? byDay.map(([d, es]) => `<div class="panel"><div class="panel-h"><h3>${esc(d)}</h3></div><div class="log">${es.map(e => `<div class="lrow"><time datetime="${new Date(e.at).toISOString()}">${esc(new Date(e.at).toLocaleTimeString(lang(), { hour: "2-digit", minute: "2-digit" }))}</time><span aria-hidden="true" style="color:${e.kind === "erro" || e.kind === "suspeito" ? "var(--bad)" : e.kind === "conflito" ? "var(--warn)" : "var(--muted)"}">${kindIcon[e.kind] || "·"}</span><span>${e.game ? `<a href="#jogo/${esc(e.game)}" style="color:inherit">${esc(e.text)}</a>` : esc(e.text)}</span></div>`).join("")}</div></div>`).join("")
        : `<div class="panel"><div class="empty">${esc(t("act.empty"))}</div></div>`}`;
  };

  // ---------- Dispositivos ----------
  R.dispositivos = async () => {
    const d = await api({ call: "devices" });
    S.dev = d;
    const me = d.me;
    const pairing = S.pairing ? `<div class="panel-b" style="display:grid;gap:10px;border-top:1px solid var(--line)" aria-live="polite">
        <span class="sub">${esc(t("dev.pair.show"))}</span><span class="code">${esc(S.pairing.slice(0, 3))} ${esc(S.pairing.slice(3))}</span>
        <span class="small sub">${esc(t("dev.pair.cli", { c: S.pairing }))}</span>
        ${me.addresses.length ? `<span class="small sub">${esc(t("dev.pair.addr"))} <span class="mono">${esc(me.addresses.join(", "))}</span></span>` : ""}
        <div><button class="btn ghost" data-act="paircancel">${esc(t("act.cancel"))}</button></div></div>` : "";
    return `<div class="pagehead"><div><h1>${esc(t("nav.devices"))}</h1><p>${esc(t("dev.sub"))}</p></div></div>
      <div class="devs">
        <div class="panel dev"><div class="panel-b" style="display:grid;gap:12px">
          <div class="top"><span class="ico" style="color:${mcolor(me.id)}" aria-hidden="true"><svg width="22" height="22" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6"><rect x="2.5" y="4" width="15" height="9" rx="1.2"/><path d="M7 16h6M10 13v3"/></svg></span><div><b>${esc(me.name)}</b> <span class="pill plain">${esc(t("side.me"))}</span><div class="small sub mono">${esc(me.fingerprint)}</div></div></div>
          <dl class="kv"><dt>${esc(t("dev.port"))}</dt><dd>${me.port ? "UDP " + me.port : esc(t("dev.p2poff"))}</dd><dt>${esc(t("dev.addrs"))}</dt><dd class="mono small">${esc(me.addresses.join(", ") || "—")}</dd><dt>${esc(t("dev.secrets"))}</dt><dd>${esc(d.secrets_backend)}</dd></dl>
          <div class="actions"><button class="btn primary" data-act="pairstart" ${me.port ? "" : "disabled"}>${esc(t("dev.pair.start"))}</button><button class="btn" data-act="pairjoin">${esc(t("dev.pair.join"))}</button></div>
        </div>${pairing}</div>
        <div class="panel"><div class="panel-h"><h3>${esc(t("dev.peers"))}</h3><button class="btn ghost" data-act="peerssync">${esc(t("act.syncnow"))}</button></div>
          <div class="dests">${d.peers.map(p => `<div class="dest"><span class="di" style="color:${mcolor(p.machine)}" aria-hidden="true">⇄</span><div><b>${esc(p.name)}</b><div class="small sub">${esc(p.nearby ? t("dev.nearby") : p.updated_at ? t("side.seen", { w: fmtAgo(p.updated_at) }) : t("dev.never"))}${p.addresses.length ? " · " + esc(p.addresses.join(", ")) : ""}</div></div><button class="btn ghost danger" data-act="peerremove" data-m="${esc(p.machine)}" data-n="${esc(p.name)}">${esc(t("act.forget"))}</button></div>`).join("") || `<div class="panel-b sub">${esc(t("dev.nopeers"))}</div>`}
          ${d.nearby.map(n => `<div class="dest"><span class="di" aria-hidden="true">?</span><div><b class="mono">${esc(n.machine)}</b><div class="small sub">${esc(t("dev.unpaired"))} · ${esc(n.addrs.join(", "))}</div></div><span></span></div>`).join("")}</div></div>
      </div>
      <div class="panel"><div class="panel-h"><h3>${esc(t("dev.clouds"))}</h3><div class="actions"><button class="btn" data-act="cloudsync" ${d.clouds.length ? "" : "disabled"}>${esc(t("act.syncnow"))}</button><button class="btn primary" data-act="cloudadd">${esc(t("dev.cloud.add"))}</button></div></div>
        <div class="dests">${d.clouds.map(c => `<div class="dest"><span class="di" aria-hidden="true">☁</span><div><b>${esc(c.name)}</b> ${c.encrypted ? `<span class="pill ok">${esc(t("dev.cloud.enc"))}</span>` : `<span class="pill warn">${esc(t("dev.cloud.plain"))}</span>`}<div class="small sub">${esc(c.describe)}${c.last_check ? " · " + esc(t("dev.cloud.checked", { w: fmtAgo(c.last_check) })) : ""}</div></div><div class="actions"><button class="btn ghost" data-act="cloudcheck" data-n="${esc(c.name)}">${esc(t("dev.cloud.check"))}</button><button class="btn ghost danger" data-act="cloudremove" data-n="${esc(c.name)}">${esc(t("act.remove"))}</button></div></div>`).join("") || `<div class="panel-b sub">${esc(t("dev.noclouds"))}</div>`}</div></div>`;
  };

  function cloudForm(gdrive) {
    return `<div class="panel-h"><h3>${esc(t("dev.cloud.add"))}</h3></div><form class="panel-b" id="cloudform">
      <div class="fields">
        <div class="field"><label for="cf-kind">${esc(t("cf.kind"))}</label><select id="cf-kind"><option value="pasta">${esc(t("cf.pasta"))}</option><option value="webdav">WebDAV (Nextcloud, ownCloud…)</option><option value="s3">S3 (R2, B2, MinIO…)</option>${gdrive ? `<option value="gdrive">Google Drive</option>` : ""}</select></div>
        <div class="field"><label for="cf-name">${esc(t("cf.name"))}</label><input id="cf-name" required value="nuvem"></div>
        <div data-k="pasta" class="field"><label for="cf-path">${esc(t("cf.path"))}</label><div style="display:flex;gap:8px"><input id="cf-path"><button type="button" class="btn" data-act="cfbrowse">${esc(t("act.browse"))}</button></div><span class="hint">${esc(t("cf.path.hint"))}</span></div>
        <div data-k="webdav" class="field" hidden><label for="cf-url">URL</label><input id="cf-url" placeholder="https://nuvem.exemplo/remote.php/dav/files/usuario/"></div>
        <div data-k="webdav" class="field" hidden><label for="cf-user">${esc(t("cf.user"))}</label><input id="cf-user" autocomplete="username"></div>
        <div data-k="webdav" class="field" hidden><label for="cf-wpass">${esc(t("cf.wpass"))}</label><input id="cf-wpass" type="password" autocomplete="current-password"></div>
        <div data-k="s3" class="field" hidden><label for="cf-bucket">Bucket</label><input id="cf-bucket"></div>
        <div data-k="s3" class="field" hidden><label for="cf-endpoint">Endpoint</label><input id="cf-endpoint" placeholder="${esc(t("cf.endpoint.hint"))}"></div>
        <div data-k="s3" class="field" hidden><label for="cf-ak">Access key</label><input id="cf-ak"></div>
        <div data-k="s3" class="field" hidden><label for="cf-sk">Secret key</label><input id="cf-sk" type="password"></div>
        <div data-k="gdrive" class="field" hidden><span class="hint">${esc(t("cf.gdrive.hint"))}</span></div>
        <div class="setrow" style="padding:0;border:0"><div><b>${esc(t("cf.encrypt"))}</b><p>${esc(t("cf.encrypt.hint"))}</p></div><button type="button" class="switch" role="switch" aria-checked="true" id="cf-enc" aria-label="${esc(t("cf.encrypt"))}"></button></div>
        <div class="field" id="cf-pwbox"><label for="cf-pw">${esc(t("cf.password"))}</label><input id="cf-pw" type="password" minlength="8" autocomplete="new-password"><span class="hint">${esc(t("cf.password.hint"))}</span></div>
        <p class="err small" id="cf-err" role="alert"></p>
      </div>
      <div class="actions"><button class="btn primary" type="submit">${esc(t("act.connect"))}</button><button class="btn ghost" type="button" data-act="closemodal">${esc(t("act.cancel"))}</button></div>
    </form>`;
  }
  function readCloudForm() {
    const k = $("#cf-kind").value, v = (id) => $("#" + id).value.trim();
    let spec, secret = null;
    if (k === "pasta") spec = { kind: "pasta", path: v("cf-path") };
    else if (k === "webdav") { spec = { kind: "webdav", endpoint: v("cf-url"), root: "/Relicario", user: v("cf-user") }; secret = $("#cf-wpass").value || null; }
    else if (k === "s3") { spec = { kind: "s3", bucket: v("cf-bucket"), endpoint: v("cf-endpoint"), region: "auto", root: "relicario", access_key: v("cf-ak") }; secret = $("#cf-sk").value; }
    else spec = { kind: "gdrive", root: "/Relicario", client_id: "" };
    const enc = $("#cf-enc").getAttribute("aria-checked") === "true";
    return { call: "cloud_add", cloud: { name: v("cf-name"), spec, encrypted: enc }, secret, password: enc ? $("#cf-pw").value : null };
  }
  async function addCloud(form, after) {
    const req = readCloudForm();
    if (req.cloud.encrypted && (req.password || "").length < 8) { $("#cf-err").textContent = t("cf.password.short"); return; }
    $("#cf-err").textContent = "";
    try {
      const r = await busy(form.querySelector("[type=submit]"), api(req));
      if (r.recovery) {
        $("#modal .modal").innerHTML = `<div class="panel-h"><h3>${esc(t("rec.title"))}</h3></div><div class="panel-b"><p>${esc(t("rec.sub"))}</p><div class="recovery">${esc(r.recovery)}</div><div class="actions"><button class="btn primary" data-act="copyrec">${esc(t("act.copy"))}</button><button class="btn" data-act="closemodal">${esc(t("rec.saved"))}</button></div></div>`;
        $("#modal [data-act=copyrec]").onclick = () => navigator.clipboard.writeText(r.recovery).then(() => toast(t("copied")));
      } else { closeModal(); toast(r.new === false ? t("cf.joined") : t("cf.added")); }
      if (after) after();
    } catch (e) { $("#cf-err").textContent = String(e); }
  }
  function mountCloudForm(m) {
    const sel = $("#cf-kind", m);
    const upd = () => $$("[data-k]", m).forEach(el => el.hidden = el.dataset.k !== sel.value);
    sel.onchange = upd; upd();
    $("#cf-enc", m).onclick = (e) => { const on = e.currentTarget.getAttribute("aria-checked") !== "true"; e.currentTarget.setAttribute("aria-checked", on); $("#cf-pwbox", m).hidden = !on; };
  }

  // ---------- Ajustes ----------
  R.ajustes = async () => {
    const s = await api({ call: "settings" });
    S.set = s;
    const r = s.retention;
    const langSel = store.get("lang") || "auto";
    const theme = store.get("theme") || "auto";
    return `<div class="pagehead"><div><h1>${esc(t("nav.settings"))}</h1><p class="mono small">${esc(s.config_path)}</p></div></div>
      <div class="setgrid"><nav class="setnav" aria-label="${esc(t("set.sections"))}"><a href="#ajustes" data-scroll="set-geral">${esc(t("set.general"))}</a><a href="#ajustes" data-scroll="set-hist">${esc(t("set.history"))}</a><a href="#ajustes" data-scroll="set-arte">${esc(t("set.art"))}</a><a href="#ajustes" data-scroll="set-ign">${esc(t("set.ignored"))}</a></nav>
      <div style="display:grid;gap:16px">
        <div class="panel" id="set-geral"><div class="panel-h"><h3>${esc(t("set.general"))}</h3></div>
          <div class="setrow"><div><b>${esc(t("set.name"))}</b><p>${esc(t("set.name.hint"))}</p></div><input class="inp" id="s-name" value="${esc(s.machine_name)}" aria-label="${esc(t("set.name"))}"></div>
          <div class="setrow"><div><b>${esc(t("set.lang"))}</b></div><select class="inp" id="s-lang" aria-label="${esc(t("set.lang"))}"><option value="auto" ${langSel === "auto" ? "selected" : ""}>${esc(t("set.auto"))}</option><option value="pt-BR" ${langSel === "pt-BR" ? "selected" : ""}>Português (Brasil)</option><option value="en" ${langSel === "en" ? "selected" : ""}>English</option></select></div>
          <div class="setrow"><div><b>${esc(t("set.theme"))}</b></div><div class="seg" role="group" aria-label="${esc(t("set.theme"))}">${["auto", "dark", "light"].map(x => `<button data-theme-set="${x}" aria-pressed="${theme === x}">${esc(t("set.theme." + x))}</button>`).join("")}</div></div>
          <div class="setrow"><div><b>${esc(t("set.autodetect"))}</b><p>${esc(t("set.autodetect.hint"))}</p></div><button class="switch" role="switch" id="s-auto" aria-checked="${s.auto_detect}" aria-label="${esc(t("set.autodetect"))}"></button></div>
          <div class="setrow"><div><b>${esc(t("set.periodic"))}</b><p>${esc(t("set.periodic.hint"))}</p></div><select class="inp" id="s-periodic" aria-label="${esc(t("set.periodic"))}">${[900, 1800, 3600, 7200, 21600].map(v => `<option value="${v}" ${s.periodic_secs === v ? "selected" : ""}>${v < 3600 ? v / 60 + " min" : v / 3600 + " h"}</option>`).join("")}</select></div>
          <div class="setrow"><div><b>${esc(t("set.p2p"))}</b><p>${esc(t("set.p2p.hint"))}</p></div><input class="inp" id="s-port" type="number" min="0" max="65535" value="${s.p2p_port}" style="width:110px" aria-label="${esc(t("set.p2p"))}"></div>
          ${s.clouds.map(c => `<div class="setrow"><div><b>${esc(t("set.limit", { n: c.name }))}</b><p>${esc(t("set.limit.hint"))}</p></div><input class="inp" data-limit="${esc(c.name)}" type="number" min="0" value="${c.limit_kbps || 0}" style="width:110px" aria-label="${esc(t("set.limit", { n: c.name }))}"></div>`).join("")}
        </div>
        <div class="panel" id="set-hist"><div class="panel-h"><h3>${esc(t("set.history"))}</h3><button class="btn" data-act="prune">${esc(t("act.prune"))}</button></div>
          <div class="panel-b sub small">${esc(t("set.ret.hint"))}</div>
          <div class="retention">
            <div><label class="small sub" for="r-all">${esc(t("set.ret.all"))}</label><input class="inp" id="r-all" type="number" min="1" value="${r.all_days}" style="width:100%"></div>
            <div><label class="small sub" for="r-daily">${esc(t("set.ret.daily"))}</label><input class="inp" id="r-daily" type="number" min="1" value="${r.daily_days}" style="width:100%"></div>
            <div><label class="small sub" for="r-monthly">${esc(t("set.ret.monthly"))}</label><input class="inp" id="r-monthly" type="number" min="1" value="${r.monthly_days}" style="width:100%"></div>
            <div><b>📌</b><span>${esc(t("set.ret.pins"))}</span></div>
          </div></div>
        <div class="panel" id="set-arte"><div class="panel-h"><h3>${esc(t("set.art"))}</h3></div>
          <div class="setrow"><div><b>${esc(t("set.sgdb"))}</b><p>${esc(t("set.sgdb.hint"))} ${s.has_steamgriddb_key ? `<span class="pill ok">${esc(t("set.sgdb.ok"))}</span>` : ""}</p></div><input class="inp" id="s-sgdb" type="password" placeholder="${esc(s.has_steamgriddb_key ? "••••••••" : t("set.sgdb.ph"))}" aria-label="${esc(t("set.sgdb"))}"></div></div>
        <div class="panel" id="set-ign"><div class="panel-h"><h3>${esc(t("set.ignored"))}</h3></div>
          ${s.ignored.map(g => `<div class="setrow"><div><b>${esc(g)}</b></div><button class="btn ghost" data-act="unignore" data-g="${esc(g)}">${esc(t("set.unignore"))}</button></div>`).join("") || `<div class="panel-b sub">${esc(t("set.noignored"))}</div>`}</div>
        <div class="actions"><button class="btn primary" data-act="savesettings">${esc(t("act.save"))}</button></div>
      </div></div>`;
  };

  // ---------- Assistente (primeira execução e máquina nova) ----------
  const W = { step: 0, name: "", repo: "", way: "par", st: null };
  async function wizard() {
    $("#shell").hidden = true;
    const el = $("#wizard"); el.hidden = false;
    if (!W.st) W.st = await invoke("app_status").catch(() => ({ hostname: "", default_repo: "" }));
    if (!W.name) W.name = W.st.hostname || "";
    const steps = [t("wz.s1"), t("wz.s2"), t("wz.s3"), t("wz.s4")];
    const head = `<div class="steps" aria-label="${esc(t("wz.progress"))}">${steps.map((s, i) => `<div class="step ${i < W.step ? "done" : i === W.step ? "now" : ""}" ${i === W.step ? 'aria-current="step"' : ""}><i>${i < W.step ? "✓" : i + 1}</i>${esc(s)}</div>`).join("")}</div>`;
    let body;
    if (W.step === 0) body = `<div class="panel"><div class="panel-b" style="display:grid;gap:14px">
        <h1>${esc(t("wz.welcome"))}</h1><p class="sub" style="margin:0">${esc(t("wz.welcome.sub"))}</p>
        <div class="field"><label for="w-name">${esc(t("wz.name"))}</label><input id="w-name" value="${esc(W.name)}"><span class="hint">${esc(t("wz.name.hint"))}</span></div>
        <div class="field"><label for="w-repo">${esc(t("wz.repo"))}</label><div style="display:flex;gap:8px"><input id="w-repo" value="${esc(W.repo)}" placeholder="${esc(W.st.default_repo)}"><button class="btn" data-act="wbrowse">${esc(t("act.browse"))}</button></div><span class="hint">${esc(t("wz.repo.hint"))}</span></div>
        <p class="err small" id="w-err" role="alert"></p>
        <div class="actions"><button class="btn primary" data-act="wnext">${esc(t("act.continue"))}</button></div></div></div>`;
    else if (W.step === 1) body = `<div class="panel"><div class="panel-b" style="display:grid;gap:14px">
        <h1>${esc(t("wz.other"))}</h1><p class="sub" style="margin:0">${esc(t("wz.other.sub"))}</p>
        <div class="opts" style="grid-template-columns:repeat(3,minmax(0,1fr))" role="group">${[["par", t("wz.way.par"), t("wz.way.par.sub")], ["nuvem", t("wz.way.nuvem"), t("wz.way.nuvem.sub")], ["zero", t("wz.way.zero"), t("wz.way.zero.sub")]].map(o => `<button class="opt" data-way="${o[0]}" aria-pressed="${W.way === o[0]}"><b>${esc(o[1])}</b><span>${esc(o[2])}</span></button>`).join("")}</div>
        ${W.way === "par" ? `<div class="field"><label for="w-code">${esc(t("wz.code"))}</label><input id="w-code" inputmode="numeric" autocomplete="one-time-code" placeholder="123 456" maxlength="7"><span class="hint">${esc(t("wz.code.hint"))}</span></div>
          <div class="field"><label for="w-addr">${esc(t("wz.addr"))}</label><input id="w-addr" placeholder="192.168.0.10:47321"></div>` : ""}
        ${W.way === "nuvem" ? `<div id="w-cloud"></div>` : ""}
        <p class="err small" id="w-err" role="alert"></p>
        <div class="actions"><button class="btn primary" data-act="wconnect">${esc(W.way === "zero" ? t("act.continue") : t("act.connect"))}</button><button class="btn ghost" data-act="wback">${esc(t("act.back"))}</button></div></div></div>`;
    else if (W.step === 2) {
      const ov = await api({ call: "overview" }).catch(() => null);
      const games = ov ? ov.games : [];
      body = `<div class="panel"><div class="panel-b" style="display:grid;gap:14px">
        <h1>${esc(t("wz.games"))}</h1><p class="sub" style="margin:0">${esc(t("wz.games.sub"))}</p></div>
        <div>${games.map(g => `<div class="pickrow" style="grid-template-columns:44px minmax(0,1fr) auto">${poster(g, true)}<span><b>${esc(g.title)}</b><span class="small sub" style="display:block">${esc(g.installed ? (g.snapshots ? t("wz.g.hist", { n: g.snapshots, m: g.last_machine || "" }) : t("wz.g.new")) : t("wz.g.notinstalled"))}</span></span>${pill(g.state, g)}</div>`).join("") || `<div class="panel-b sub">${esc(t("wz.nogames"))}</div>`}</div>
        <div class="panel-b"><p class="err small" id="w-err" role="alert"></p><div class="actions"><button class="btn primary" data-act="wsync">${esc(t("wz.bring"))}</button><button class="btn ghost" data-act="wrescan">${esc(t("act.rescan"))}</button></div></div></div>`;
    } else body = `<div class="panel"><div class="panel-b" style="display:grid;gap:14px">
        <h1>${esc(t("wz.done"))}</h1><p class="sub" style="margin:0">${esc(t("wz.done.sub"))}</p>
        <div class="field"><span class="hint">${esc(t("wz.launcher"))}</span><span class="mono">relicario run -- %command%</span></div>
        <div class="actions"><button class="btn primary" data-act="wfinish">${esc(t("wz.open"))}</button></div></div></div>`;
    el.innerHTML = `<div class="wizard" style="margin:0 auto">${head}${body}</div>`;
    hydrateArt(el);
    if (W.step === 1 && W.way === "nuvem") {
      const box = $("#w-cloud");
      box.innerHTML = `<div class="panel">${cloudForm(W.gdrive)}</div>`;
      $("#cloudform [data-act=closemodal]").remove();
      mountCloudForm(box);
      $("#cloudform").onsubmit = (e) => { e.preventDefault(); addCloud(e.target, () => { W.step = 2; wizard(); }); };
      $("#w-cloud").closest(".panel").querySelector(".panel-h").remove();
      $$("[data-act=wconnect]").forEach(b => b.hidden = true);
    }
    const f = el.querySelector("input"); if (f) f.focus(); else el.focus();
  }
  async function wizardAction(act, btn) {
    const err = (e) => { const x = $("#w-err"); if (x) x.textContent = String(e); };
    if (act === "wbrowse") { const p = await invoke("pick_folder"); if (p) $("#w-repo").value = p; return; }
    if (act === "wnext") {
      W.name = $("#w-name").value.trim(); W.repo = $("#w-repo").value.trim();
      try { await busy(btn, invoke("setup", { name: W.name, repo: W.repo || null })); } catch (e) { return err(e); }
      const d = await api({ call: "devices" }).catch(() => ({}));
      W.gdrive = !!d.gdrive_available;
      W.step = 1; return wizard();
    }
    if (act === "wback") { W.step = 0; return wizard(); }
    if (act === "wconnect") {
      if (W.way === "zero") { W.step = 2; return wizard(); }
      if (W.way === "par") {
        const code = $("#w-code").value.replace(/\D/g, ""), addr = $("#w-addr").value.trim() || null;
        try { await busy(btn, api({ call: "pair_join", code, addr })); } catch (e) { return err(e); }
        W.step = 2; return wizard();
      }
    }
    if (act === "wrescan") { await busy(btn, daemon({ op: "rescan" })).catch(err); return wizard(); }
    if (act === "wsync") {
      try { await busy(btn, daemon({ op: "sync", game: null })); } catch (e) { return err(e); }
      W.step = 3; return wizard();
    }
    if (act === "wfinish") { location.hash = "#biblioteca"; return start(); }
  }

  // ---------- ações ----------
  async function act(name, el) {
    const { name: rn, arg } = route();
    switch (name) {
      case "syncall": {
        const r = await busy(el, daemon({ op: "sync", game: null })).catch(fail);
        if (r && r.messages) toast(r.messages.length ? r.messages.join(" · ") : t("top.ok"));
        S.games.clear(); await refreshOverview(); return render();
      }
      case "syncgame": {
        const r = await busy(el, daemon({ op: "sync", game: arg })).catch(fail);
        if (r && r.messages) toast(r.messages.join(" · "));
        S.games.delete(arg); await refreshOverview(); return render();
      }
      case "rescan": { await busy(el, daemon({ op: "rescan" })).then(() => toast(t("toast.rescanned"))).catch(fail); await refreshOverview(); return render(); }
      case "restore": {
        const g = S.games.get(arg), s = g.snapshots.find(x => x.id === el.dataset.sid);
        if (!await confirmBox(t("restore.q", { s: s.short, w: fmtWhen(s.created_at) }), t("restore.sub"), t("act.restore"))) return;
        const r = await busy(el, api({ call: "restore", game: arg, snapshot: s.id })).catch(fail);
        if (r) { toast(r.text); S.snaps.clear(); return render(); }
        return;
      }
      case "pin": {
        const s = S.games.get(arg).snapshots.find(x => x.id === el.dataset.sid);
        const n = await promptBox(t("pin.q"), s.pinned || "", t("act.pin"));
        if (n == null || !n.trim()) return;
        await api({ call: "pin", snapshot: s.id, name: n.trim() }).then(() => toast(t("toast.pinned"))).catch(fail);
        return render();
      }
      case "unpin": await api({ call: "unpin", snapshot: el.dataset.sid }).catch(fail); return render();
      case "zip": {
        const g = S.games.get(arg), s = g.snapshots.find(x => x.id === el.dataset.sid);
        const dest = await invoke("pick_save_path", { name: `${g.id}-${s.short}.zip` }).catch(fail);
        if (!dest) return;
        const r = await busy(el, api({ call: "export_zip", game: g.id, snapshot: s.id, dest })).catch(fail);
        if (r) toast(t("toast.zip", { n: r.files }));
        return;
      }
      case "picker": S.picker = !S.picker; return render();
      case "artreset": await api({ call: "art_reset", game: arg, kind: S.pickKind }).catch(fail); S.art.delete(`${arg}:${S.pickKind}`); return render();
      case "artfile": {
        const p = await invoke("pick_image").catch(fail); if (!p) return;
        await api({ call: "art_file", game: arg, kind: S.pickKind, path: p }).catch(fail);
        S.art.delete(`${arg}:${S.pickKind}`); return render();
      }
      case "resolve": {
        if (S.choice === "later") { location.hash = "#biblioteca"; return; }
        const r = await busy(el, api({ call: "resolve", game: arg, keep: S.pick, keep_both: S.choice === "both" })).catch(fail);
        if (r) { toast(r.text); S.pick = null; S.snaps.clear(); S.games.delete(arg); await refreshOverview(); location.hash = `#jogo/${arg}`; }
        return;
      }
      case "prune": {
        if (!await confirmBox(t("prune.q"), t("prune.sub"), t("act.prune"))) return;
        const r = await busy(el, api({ call: "prune" })).catch(fail);
        if (r) toast(t("toast.pruned", { n: r.forgotten, s: fmtSize(r.bytes_freed) }));
        return;
      }
      case "pairstart": {
        const code = await invoke("new_pair_code");
        try { await daemon({ op: "pair_start", code }); } catch (e) { return fail(e); }
        S.pairing = code; render(); pollPairing(); return;
      }
      case "paircancel": S.pairing = null; await daemon({ op: "pair_cancel" }).catch(() => { }); return render();
      case "pairjoin": {
        modal(`<div class="panel-h"><h3>${esc(t("dev.pair.join"))}</h3></div><form class="panel-b" id="joinform">
          <div class="field"><label for="j-code">${esc(t("wz.code"))}</label><input id="j-code" inputmode="numeric" maxlength="7" required></div>
          <div class="field"><label for="j-addr">${esc(t("wz.addr"))}</label><input id="j-addr" placeholder="192.168.0.10:47321"></div>
          <p class="err small" id="j-err" role="alert"></p>
          <div class="actions"><button class="btn primary" type="submit">${esc(t("act.connect"))}</button><button class="btn ghost" type="button" data-act="closemodal">${esc(t("act.cancel"))}</button></div></form>`, (m) => {
          $("#joinform", m).onsubmit = async (e) => {
            e.preventDefault();
            try { const r = await busy(e.target.querySelector("[type=submit]"), api({ call: "pair_join", code: $("#j-code").value, addr: $("#j-addr").value.trim() || null })); closeModal(); toast(t("toast.paired", { m: r.name })); render(); } catch (x) { $("#j-err").textContent = String(x); }
          };
        });
        return;
      }
      case "peerssync": { const r = await busy(el, daemon({ op: "peers_sync" })).catch(fail); if (r) toast((r.messages || []).join(" · ") || t("top.ok")); return render(); }
      case "peerremove": if (await confirmBox(t("peer.q", { m: el.dataset.n }), t("peer.sub"), t("act.forget"))) { await api({ call: "peer_remove", machine: el.dataset.m }).catch(fail); render(); } return;
      case "cloudadd": modal(cloudForm(S.dev && S.dev.gdrive_available), (m) => { mountCloudForm(m); $("#cloudform", m).onsubmit = (e) => { e.preventDefault(); addCloud(e.target, () => render()); }; }); return;
      case "cfbrowse": { const p = await invoke("pick_folder"); if (p) $("#cf-path").value = p; return; }
      case "cloudsync": { const r = await busy(el, api({ call: "cloud_sync" })).catch(fail); if (r) toast(r.map(x => x.message).join(" · ")); return render(); }
      case "cloudcheck": { const r = await busy(el, api({ call: "cloud_check", name: el.dataset.n })).catch(fail); if (r) toast(t("toast.checked", { n: el.dataset.n })); return render(); }
      case "cloudremove": if (await confirmBox(t("cloud.q", { n: el.dataset.n }), t("cloud.sub"), t("act.remove"))) { await api({ call: "cloud_remove", name: el.dataset.n }).catch(fail); render(); } return;
      case "unignore": await api({ call: "set_ignored", game: el.dataset.g, ignored: false }).catch(fail); return render();
      case "savesettings": {
        const v = (id) => $("#" + id).value;
        const limits = {}; $$("[data-limit]").forEach(i => limits[i.dataset.limit] = Number(i.value) || 0);
        const settings = {
          machine_name: v("s-name"), auto_detect: $("#s-auto").getAttribute("aria-checked") === "true",
          periodic_secs: Number(v("s-periodic")), cloud_limits: limits,
          retention: { all_days: Number(v("r-all")) || 1, daily_days: Number(v("r-daily")) || 30, monthly_days: Number(v("r-monthly")) || 365 },
        };
        if (Number(v("s-port")) !== S.set.p2p_port) settings.p2p_port = Number(v("s-port"));
        if (v("s-sgdb")) settings.steamgriddb_key = v("s-sgdb");
        const l = v("s-lang"); store.set("lang", l); window.I18N.setLang(l === "auto" ? null : l); applyStatic();
        await busy(el, api({ call: "save_settings", settings })).then(() => toast(t("toast.saved"))).catch(fail);
        S.art.clear(); await refreshOverview(); return render();
      }
      case "closemodal": return closeModal();
    }
    if (name.startsWith("w")) return wizardAction(name, el);
  }

  async function pollPairing() {
    while (S.pairing) {
      await new Promise(r => setTimeout(r, 1000));
      if (!S.pairing) return;
      let st;
      try { st = (await daemon({ op: "pair_status" })).messages[0] || ""; } catch { st = "encerrado"; }
      if (st.startsWith("pareado")) { S.pairing = null; toast(t("toast.paired", { m: st.replace(/^pareado com /, "") })); daemon({ op: "peers_sync" }).catch(() => { }); return render(); }
      if (st === "encerrado") { S.pairing = null; toast(t("toast.pairfail"), true); return render(); }
    }
  }

  function confirmBox(title, sub, ok) {
    return new Promise((res) => {
      modal(`<div class="panel-h"><h3>${esc(title)}</h3></div><div class="panel-b"><p class="sub" style="margin:0">${esc(sub)}</p><div class="actions"><button class="btn primary" data-yes>${esc(ok)}</button><button class="btn ghost" data-no>${esc(t("act.cancel"))}</button></div></div>`, (m) => {
        $("[data-yes]", m).onclick = () => { closeModal(); res(true); };
        $("[data-no]", m).onclick = () => { closeModal(); res(false); };
        $("[data-yes]", m).focus();
      });
      S.modalRes = res;
    });
  }
  function promptBox(title, value, ok) {
    return new Promise((res) => {
      modal(`<div class="panel-h"><h3>${esc(title)}</h3></div><form class="panel-b"><input class="inp" id="p-v" value="${esc(value)}" aria-label="${esc(title)}"><div class="actions"><button class="btn primary" type="submit">${esc(ok)}</button><button class="btn ghost" type="button" data-no>${esc(t("act.cancel"))}</button></div></form>`, (m) => {
        $("form", m).onsubmit = (e) => { e.preventDefault(); const v = $("#p-v").value; closeModal(); res(v); };
        $("[data-no]", m).onclick = () => { closeModal(); res(null); };
        $("#p-v").select();
      });
      S.modalRes = () => res(null);
    });
  }

  // ---------- eventos ----------
  document.addEventListener("click", (e) => {
    const x = e.target.closest("[data-go],[data-act],[data-filter],[data-libview],[data-tab],[data-snap],[data-pick],[data-choice-opt],[data-mode],[data-pickkind],[data-choice],[data-way],[data-theme-set],[data-scroll],[data-close-bg],.switch");
    if (!x) return;
    if (x.dataset.closeBg !== undefined) { if (e.target === x) { closeModal(); if (S.modalRes) S.modalRes(false); } return; }
    const { arg } = route();
    if (x.dataset.go) { location.hash = "#" + x.dataset.go; return; }
    if (x.dataset.act) { e.preventDefault(); act(x.dataset.act, x); return; }
    if (x.dataset.filter) { S.filter = x.dataset.filter; return render(); }
    if (x.dataset.libview) { S.libView = x.dataset.libview; store.set("libView", S.libView); return render(); }
    if (x.dataset.tab) { S.gameTab = x.dataset.tab; return render(); }
    if (x.dataset.snap) {
      S.sel = x.dataset.snap;
      $$(".snap").forEach(b => b.setAttribute("aria-pressed", b.dataset.snap === S.sel));
      const g = S.games.get(arg), s = g && g.snapshots.find(z => z.id === S.sel);
      if (s) { $("#snappanel").outerHTML = snapPanel(g, s, S.snaps.get(arg + ":" + s.id)); loadSnap(arg, s.id); }
      return;
    }
    if (x.dataset.pick) { S.pick = x.dataset.pick; if (S.choice === "later") S.choice = "keep"; return render(); }
    if (x.dataset.choiceOpt) { S.choice = x.dataset.choiceOpt; return render(); }
    if (x.dataset.mode) {
      const m = x.dataset.mode;
      const p = m === "ignorar" ? api({ call: "set_ignored", game: arg, ignored: true })
        : api({ call: "set_ignored", game: arg, ignored: false }).then(() => api({ call: "set_mode", game: arg, mode: m }));
      p.then(() => { toast(t("toast.mode")); S.ov = null; return render(); }).catch(fail);
      return;
    }
    if (x.dataset.pickkind) { S.pickKind = x.dataset.pickkind; return render(); }
    if (x.dataset.choice !== undefined && S.choices) {
      const c = S.choices[Number(x.dataset.choice)];
      busy(x, api({ call: "art_set", game: arg, kind: S.pickKind, choice: c })).then(() => { S.art.delete(`${arg}:${S.pickKind}`); toast(t("toast.art")); render(); }).catch(fail);
      return;
    }
    if (x.dataset.way) { W.way = x.dataset.way; return wizard(); }
    if (x.dataset.themeSet) { store.set("theme", x.dataset.themeSet); applyTheme(); $$("[data-theme-set]").forEach(b => b.setAttribute("aria-pressed", b === x)); return; }
    if (x.dataset.scroll) { e.preventDefault(); const tg = document.getElementById(x.dataset.scroll); if (tg) tg.scrollIntoView({ behavior: "smooth" }); return; }
    if (x.classList.contains("switch") && x.id !== "cf-enc") { x.setAttribute("aria-checked", x.getAttribute("aria-checked") !== "true"); }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && $("#modal").innerHTML) { closeModal(); if (S.modalRes) S.modalRes(false); return; }
    if (e.key === "/" && !/INPUT|SELECT|TEXTAREA/.test(document.activeElement.tagName)) { e.preventDefault(); const q = $("#q"); if (q) q.focus(); }
  });
  let qTimer;
  document.addEventListener("input", (e) => {
    if (e.target.id !== "q") return;
    clearTimeout(qTimer);
    qTimer = setTimeout(() => { S.q = e.target.value; if (route().name !== "biblioteca") location.hash = "#biblioteca"; else render(); }, 150);
  });
  window.addEventListener("hashchange", () => { S.picker = false; if (route().name !== "jogo") S.sel = null; $("#view").scrollTop = 0; render().then(() => $("#view").focus({ preventScroll: true })); });

  // ---------- tema e textos fixos ----------
  function applyTheme() {
    const th = store.get("theme") || "auto";
    if (th === "auto") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", th);
  }
  function applyStatic() {
    document.documentElement.lang = window.I18N.lang();
    $$("[data-t]").forEach(el => el.textContent = t(el.dataset.t));
    $$("[data-tp]").forEach(el => el.placeholder = t(el.dataset.tp));
    $$("[data-ta]").forEach(el => el.setAttribute("aria-label", t(el.dataset.ta)));
    $("#skip").textContent = t("skip");
  }

  // ---------- início ----------
  let timer;
  async function start() {
    applyTheme(); applyStatic();
    let st;
    try { st = await invoke("app_status"); } catch (e) {
      $("#wizard").hidden = false; $("#wizard").innerHTML = `<div class="empty err">${esc(e)}</div>`; return;
    }
    if (!st.configured || location.hash === "#nova-maquina") { W.st = st; return wizard(); }
    $("#wizard").hidden = true; $("#shell").hidden = false;
    await refreshOverview();
    await render();
    clearInterval(timer);
    // Atualiza o estado a cada 5 s; redesenha a biblioteca se algo mudou.
    timer = setInterval(async () => {
      if (document.hidden || $("#modal").innerHTML) return;
      const before = JSON.stringify(S.ov && S.ov.games.map(g => [g.id, g.state, g.snapshots, g.running_elsewhere]));
      await refreshOverview();
      const after = JSON.stringify(S.ov && S.ov.games.map(g => [g.id, g.state, g.snapshots, g.running_elsewhere]));
      if (before !== after && ["biblioteca", "conflitos"].includes(route().name) && document.activeElement.id !== "q") render();
    }, 5000);
  }
  start();
})();
