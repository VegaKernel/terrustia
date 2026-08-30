// Thin client for the panel's backend API. No framework beyond fetch/WebSocket.

const SESSION_KEY = "terrustia_panel_session";

export function storedSession(): string | null {
  try {
    return localStorage.getItem(SESSION_KEY);
  } catch {
    return null;
  }
}

function setSession(token: string | null) {
  try {
    if (token) localStorage.setItem(SESSION_KEY, token);
    else localStorage.removeItem(SESSION_KEY);
  } catch {
    // A private window or blocked storage just means the session doesn't survive a reload —
    // not fatal, the panel still works for the current tab session.
  }
}

export interface LoginRequest {
  name: string;
  password: string;
  /** Only required (and only checked) while the server is unclaimed. */
  claim_token?: string;
}

export interface LoginResponse {
  session: string;
  name: string;
}

export interface ApiError {
  error: string;
}

export class ApiCallError extends Error {}

async function unwrap<T>(res: Response): Promise<T> {
  if (!res.ok) {
    let message = `request failed (${res.status})`;
    try {
      const body = (await res.json()) as ApiError;
      if (body.error) message = body.error;
    } catch {
      // Non-JSON error body — the status code is all we get.
    }
    if (res.status === 401) setSession(null);
    throw new ApiCallError(message);
  }
  // Several endpoints answer a successful mutation with an empty `200 OK` and no body (kick, ban,
  // motd, world switch, console, chat, save, and the account mutations). Calling `res.json()` on an
  // empty body throws "Unexpected end of JSON input", so read the text first and only parse when
  // there is something to parse — a void-returning caller simply ignores the `undefined`.
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

function authHeaders(session: string): HeadersInit {
  return { authorization: `Bearer ${session}` };
}

async function getJson<T>(path: string, session: string): Promise<T> {
  const res = await fetch(path, { headers: authHeaders(session) });
  return unwrap<T>(res);
}

async function postJson<T>(path: string, session: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { ...authHeaders(session), "content-type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  return unwrap<T>(res);
}

/** Whether the server has no accounts yet — decides which login form to show, before any session exists. */
export async function fetchUnclaimed(): Promise<boolean> {
  const res = await fetch("/api/unclaimed");
  const data = await unwrap<{ unclaimed: boolean }>(res);
  return data.unclaimed;
}

export async function login(req: LoginRequest): Promise<LoginResponse> {
  const res = await fetch("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(req),
  });
  const data = await unwrap<LoginResponse>(res);
  setSession(data.session);
  return data;
}

export function logout() {
  setSession(null);
}

export interface StatusResponse {
  uptime_secs: number;
  player_count: number;
  max_players: number;
  world_name: string;
  world_file: string | null;
  version: string;
  unclaimed: boolean;
  /** The signed-in account's own permission strings. A UX convenience for choosing which tabs and
   *  buttons to show; every route still re-checks its own permission on the backend regardless of
   *  what this says, so getting this wrong client-side is a display bug, never a security one. */
  permissions: string[];
  /** How many world saves have failed in a row. `0` is healthy and should show nothing; anything
   *  else means the world on disk is older than the world being played. */
  save_failures: number;
}

export async function fetchStatus(session: string): Promise<StatusResponse> {
  return getJson<StatusResponse>("/api/status", session);
}

/**
 * Whether `held` (a group's raw permission set, exactly as `/api/status`'s `permissions` field or
 * `GroupInfo.permissions` carries it) grants `want` — the bare `*`, an exact match, or a family
 * wildcard on a segment boundary (`server.*` grants `server.kick`, not `serverish.kick`). Mirrors
 * `admin::group::grants` on the backend exactly; kept in sync by hand since this workspace has no
 * shared Rust/TS codegen — see that function's own doc comment for the matching rule.
 */
export function hasPermission(held: string[], want: string): boolean {
  if (held.includes("*") || held.includes(want)) return true;
  let end = want.length;
  for (;;) {
    const dot = want.lastIndexOf(".", end - 1);
    if (dot < 0) return false;
    if (held.includes(`${want.slice(0, dot)}.*`)) return true;
    end = dot;
  }
}

// ---- live status + console/chat feed, over one WebSocket ------------------------------------

export type ConsoleLineKind = "log" | "reply" | "chat";

export interface ConsoleFeedLine {
  line_kind: ConsoleLineKind;
  level: string;
  text: string;
}

type WsFrame = ({ type: "status" } & StatusResponse) | ({ type: "console" } & ConsoleFeedLine);

function wsUrl(path: string, session: string): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}${path}?session=${encodeURIComponent(session)}`;
}

/**
 * Live status pushes and console/chat lines over the panel's one WebSocket. Reconnects on its own
 * if the connection drops.
 */
export function watchStatus(
  session: string,
  onStatus: (s: StatusResponse) => void,
  onConsoleLine: (line: ConsoleFeedLine) => void,
  onConnectionChange: (live: boolean) => void,
): () => void {
  let closed = false;
  let socket: WebSocket | null = null;

  const connect = () => {
    if (closed) return;
    socket = new WebSocket(wsUrl("/api/ws", session));
    socket.onopen = () => onConnectionChange(true);
    socket.onmessage = (event) => {
      try {
        const frame = JSON.parse(event.data) as WsFrame;
        if (frame.type === "status") onStatus(frame);
        else if (frame.type === "console") onConsoleLine(frame);
      } catch {
        // A malformed frame is a server bug, not something the panel should crash over.
      }
    };
    socket.onclose = () => {
      onConnectionChange(false);
      if (!closed) setTimeout(connect, 2000);
    };
    socket.onerror = () => socket?.close();
  };
  connect();

  return () => {
    closed = true;
    socket?.close();
  };
}

// ---- players: list, kick, ban --------------------------------------------------------------

export interface Appearance {
  skin_variant: number;
  hair_style: number;
  hair_color: [number, number, number];
  skin_color: [number, number, number];
  eye_color: [number, number, number];
  shirt_color: [number, number, number];
  undershirt_color: [number, number, number];
  pants_color: [number, number, number];
  shoe_color: [number, number, number];
}

export interface Player {
  slot: number;
  name: string;
  address: string;
  life: number;
  life_max: number;
  mana: number;
  mana_max: number;
  x: number;
  y: number;
  pvp: boolean;
  appearance: Appearance | null;
  equipped: number[];
  muted: boolean;
}

export async function fetchPlayers(session: string): Promise<Player[]> {
  return getJson<Player[]>("/api/players", session);
}

export async function kickPlayer(session: string, name: string, reason: string): Promise<void> {
  await postJson("/api/players/kick", session, { name, reason });
}

/** `duration_secs` is `undefined`/omitted for a permanent mute. */
export async function mutePlayer(
  session: string,
  name: string,
  reason: string,
  duration_secs?: number,
): Promise<void> {
  await postJson("/api/players/mute", session, { name, reason, duration_secs });
}

export async function unmutePlayer(session: string, name: string): Promise<boolean> {
  const res = await postJson<{ changed: boolean }>("/api/players/unmute", session, { name });
  return res.changed;
}

export type BanKind = "name" | "ip" | "uuid";

export async function banPlayer(
  session: string,
  kind: BanKind,
  value: string,
  reason: string,
): Promise<void> {
  await postJson("/api/players/ban", session, { kind, value, reason });
}

export async function unbanPlayer(session: string, value: string): Promise<number> {
  const res = await postJson<{ removed: number }>("/api/players/unban", session, { value });
  return res.removed;
}

// ---- whitelist -------------------------------------------------------------------------------

export interface WhitelistState {
  on: boolean;
  names: string[];
}

export async function fetchWhitelist(session: string): Promise<WhitelistState> {
  return getJson<WhitelistState>("/api/whitelist", session);
}

export async function addToWhitelist(session: string, name: string): Promise<boolean> {
  const res = await postJson<{ changed: boolean }>("/api/whitelist/add", session, { name });
  return res.changed;
}

export async function removeFromWhitelist(session: string, name: string): Promise<boolean> {
  const res = await postJson<{ changed: boolean }>("/api/whitelist/remove", session, { name });
  return res.changed;
}

// ---- worlds ------------------------------------------------------------------------------------

export interface WorldEntry {
  name: string;
  size_mb: number;
  current: boolean;
}

export async function fetchWorlds(session: string): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds", session);
}

/** Restarts the server process into a different world. See the backend's own doc comment on why
 *  this is a real process restart, not a hot-swap — expect the panel to disconnect briefly. */
export async function switchWorld(session: string, name: string): Promise<void> {
  await postJson("/api/worlds/switch", session, { name });
}

// ---- settings ----------------------------------------------------------------------------------

export interface ConfigSnapshot {
  listen: string;
  max_players: number;
  world_width: number;
  world_height: number;
  motd: string;
  password_set: boolean;
  max_chat_len: number;
  idle_timeout_secs: number;
  autosave_secs: number;
  save_target: string | null;
  whitelist_on: boolean;
  whitelist_count: number;
}

export async function fetchConfig(session: string): Promise<ConfigSnapshot> {
  return getJson<ConfigSnapshot>("/api/config", session);
}

export async function setMotd(session: string, motd: string): Promise<void> {
  await postJson("/api/config/motd", session, { motd });
}

// ---- console / chat send -----------------------------------------------------------------------

export async function sendConsoleCommand(session: string, line: string): Promise<void> {
  await postJson("/api/console", session, { line });
}

export async function sendChat(session: string, text: string): Promise<void> {
  await postJson("/api/chat", session, { text });
}

// ---- the live world view -------------------------------------------------------------------

export type TileColorName =
  | "empty"
  | "dirt"
  | "stone"
  | "grass"
  | "corruption"
  | "crimson"
  | "sand"
  | "snow"
  | "ice"
  | "jungle"
  | "ore"
  | "gem"
  | "water"
  | "lava"
  | "honey"
  | "ash"
  | "other";

export interface WorldTiles {
  world_width: number;
  world_height: number;
  sample_cols: number;
  sample_rows: number;
  tiles: TileColorName[];
}

// ---- metrics ---------------------------------------------------------------------------------

export interface PhaseCost {
  name: string;
  us: number;
}

export interface Metrics {
  budget_us: number;
  cpu_us: number;
  wall_us: number;
  worst_cpu_us: number;
  phases: PhaseCost[];
  player_count: number;
  npc_count: number;
  projectile_count: number;
  item_count: number;
  ticks: number;
  memory_bytes: number | null;
}

export async function fetchMetrics(session: string): Promise<Metrics> {
  return getJson<Metrics>("/api/metrics", session);
}

// ---- backups & rollback ----------------------------------------------------------------------

export interface BackupEntry {
  index: number;
  size_mb: number;
  age_secs: number | null;
}

export interface Backups {
  saving: boolean;
  world_file: string | null;
  kept: number;
  backups: BackupEntry[];
}

export async function fetchBackups(session: string): Promise<Backups> {
  return getJson<Backups>("/api/backups", session);
}

export async function forceSave(session: string): Promise<void> {
  await postJson("/api/save", session, {});
}

/** Roll the world back to backup number `which` (1 is the most recent). This stops the server. */
export async function rollback(session: string, which: number): Promise<string> {
  const res = await postJson<{ message: string }>("/api/rollback", session, { which });
  return res.message;
}

// ---- groups & accounts -----------------------------------------------------------------------

export interface GroupInfo {
  name: string;
  permissions: string[];
  can_admin: boolean;
}

export interface AccountInfo {
  name: string;
  group: string;
  can_admin: boolean;
}

export interface AccountsState {
  groups: GroupInfo[];
  accounts: AccountInfo[];
}

export async function fetchAccounts(session: string): Promise<AccountsState> {
  return getJson<AccountsState>("/api/accounts", session);
}

export async function setAccountGroup(
  session: string,
  name: string,
  group: string,
): Promise<void> {
  await postJson("/api/accounts/group", session, { name, group });
}

export async function createAccount(
  session: string,
  name: string,
  password: string,
  group: string,
): Promise<void> {
  await postJson("/api/accounts/create", session, { name, password, group });
}

export async function deleteAccount(session: string, name: string): Promise<void> {
  await postJson("/api/accounts/delete", session, { name });
}

// ---- group permission editing -----------------------------------------------------------------

/** Every known permission name (leaves and family wildcards), for the group editor's picker.
 *  Requires `admin.groups`; a session without it gets `403` and the caller should fall back to a
 *  read-only view. */
export async function fetchKnownPermissions(session: string): Promise<string[]> {
  return getJson<string[]>("/api/permissions", session);
}

export async function setGroupPermission(
  session: string,
  group: string,
  permission: string,
  grant: boolean,
): Promise<void> {
  await postJson("/api/groups/permissions", session, { group, permission, grant });
}

// ---- audit log -----------------------------------------------------------------------------

export interface AuditEntry {
  when: number;
  issuer: string;
  action: string;
  target: string;
  detail: string;
}

export async function fetchAuditLog(session: string, n = 50): Promise<AuditEntry[]> {
  return getJson<AuditEntry[]>(`/api/audit?n=${n}`, session);
}

// ---- world creation --------------------------------------------------------------------------

export interface WorldGenStatus {
  status: "idle" | "running" | "done" | "failed";
  running: boolean;
  name: string;
  world_file: string | null;
  message: string;
  elapsed_secs: number;
}

export interface NewWorldRequest {
  name: string;
  width: number;
  height: number;
  seed?: string;
}

/** Kick off a (slow) background world generation. Returns the initial job status. */
export async function createWorld(
  session: string,
  req: NewWorldRequest,
): Promise<WorldGenStatus> {
  return postJson<WorldGenStatus>("/api/worlds/new", session, req);
}

export async function fetchWorldGenStatus(session: string): Promise<WorldGenStatus> {
  return getJson<WorldGenStatus>("/api/worlds/new/status", session);
}

// ---- the live world view -------------------------------------------------------------------

type WorldWsFrame =
  | { type: "players"; players: Player[] }
  | ({ type: "tiles" } & WorldTiles);

export function watchWorld(
  session: string,
  onPlayers: (players: Player[]) => void,
  onTiles: (tiles: WorldTiles) => void,
): () => void {
  let closed = false;
  let socket: WebSocket | null = null;

  const connect = () => {
    if (closed) return;
    socket = new WebSocket(wsUrl("/api/ws/world", session));
    socket.onmessage = (event) => {
      try {
        const frame = JSON.parse(event.data) as WorldWsFrame;
        if (frame.type === "players") onPlayers(frame.players);
        else if (frame.type === "tiles") onTiles(frame);
      } catch {
        // Ignore a malformed frame.
      }
    };
    socket.onclose = () => {
      if (!closed) setTimeout(connect, 2000);
    };
    socket.onerror = () => socket?.close();
  };
  connect();

  return () => {
    closed = true;
    socket?.close();
  };
}
