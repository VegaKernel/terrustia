// Thin client for the panel's backend API. No framework beyond fetch/WebSocket — this is a small
// surface (foundation task: login + status only) and doesn't need more.

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
    throw new ApiCallError(message);
  }
  return (await res.json()) as T;
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
  version: string;
  unclaimed: boolean;
}

export async function fetchStatus(session: string): Promise<StatusResponse> {
  const res = await fetch("/api/status", {
    headers: { authorization: `Bearer ${session}` },
  });
  if (res.status === 401) {
    setSession(null);
    throw new ApiCallError("session expired");
  }
  return unwrap<StatusResponse>(res);
}

/** Live status pushes over the panel's WebSocket. Reconnects on its own if the connection drops. */
export function watchStatus(
  session: string,
  onStatus: (s: StatusResponse) => void,
  onDisconnect: () => void,
): () => void {
  let closed = false;
  let socket: WebSocket | null = null;

  const connect = () => {
    if (closed) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    socket = new WebSocket(`${proto}://${location.host}/api/ws?session=${encodeURIComponent(session)}`);
    socket.onmessage = (event) => {
      try {
        onStatus(JSON.parse(event.data) as StatusResponse);
      } catch {
        // A malformed frame is a server bug, not something the panel should crash over.
      }
    };
    socket.onclose = () => {
      onDisconnect();
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
