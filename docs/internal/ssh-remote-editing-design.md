# SSH Remote Editing Design

## Overview

This document describes the design for SSH remote editing support in Fresh. The architecture uses a Python agent bootstrapped via `cat agent.py | ssh user@host "python3 -u -"` that implements a JSON-RPC protocol over SSH stdin/stdout. The agent handles filesystem operations and process spawning on the remote host.

## Goals

1. **Seamless Remote Editing**: Edit files on remote servers as naturally as local files
2. **Plugin Compatibility**: All plugins (live_grep, git_grep, git_log, fuzzy finder) work transparently on remote
3. **Zero Installation**: No pre-installation required on the remote host (only Python 3)
4. **Resilient Connections**: Handle network interruptions gracefully with reconnection

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│ Fresh Editor                                                        │
│  ┌──────────────────┐  ┌──────────────────┐  ┌───────────────────┐  │
│  │ RemoteFileSystem │  │ RemoteProcessMgr │  │ SshConnection     │  │
│  │ impl FileSystem  │  │ impl ProcessSpawn│  │ lifecycle manager │  │
│  └────────┬─────────┘  └────────┬─────────┘  └─────────┬─────────┘  │
│           │                     │                      │            │
│           └──────────┬──────────┴──────────────────────┤            │
│                      │                                 │            │
│              ┌───────▼────────┐                        │            │
│              │ AgentChannel   │────────────────────────┘            │
│              │ JSON-RPC mux   │                                     │
│              └───────┬────────┘                                     │
└──────────────────────┼──────────────────────────────────────────────┘
                       │ SSH stdin/stdout (JSON-RPC)
                       ▼
┌─────────────────────────────────────────────────────────────────────┐
│ Remote Host                                                         │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ Python Agent (bootstrapped via stdin)                          │ │
│  │  - File operations (read/write/dir listing/metadata)           │ │
│  │  - Process spawning (rg, git, fd, etc. for plugins)            │ │
│  │  - JSON-RPC protocol over stdin/stdout                         │ │
│  └────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

## User Flows

### Flow 1: CLI Invocation

```bash
# Open a remote file
fresh user@host:/path/to/file.rs

# Open a remote file at specific line
fresh user@host:/path/to/file.rs:42

# Open a remote directory (starts file explorer)
fresh user@host:/path/to/project

# With explicit port
fresh user@host:2222:/path/to/file.rs
```

**Parsing Logic**:
- Detect `@` followed by `:` pattern (not Windows drive letter)
- Extract user, host, optional port, and path
- Support line:col suffix after path

### Flow 2: Open Remote Command

From within Fresh, user invokes command palette → "Open Remote":

```
┌─ Open Remote ──────────────────────────────────────────┐
│ Connection: user@host:/path                            │
│                                                        │
│ Recent:                                                │
│   • user@server1:/home/user/project                    │
│   • deploy@prod:/var/www/app                           │
└────────────────────────────────────────────────────────┘
```

On confirmation:
1. Parse connection string
2. Establish SSH connection
3. Bootstrap agent
4. Open file/directory

### Flow 3: Reconnection

When connection drops:
1. Status bar shows `[SSH: RECONNECTING...]`
2. Operations queue locally
3. On reconnect, replay queued operations
4. On failure after N retries, prompt user

## Agent Protocol

A simple streaming protocol over JSON lines. Every message is one line of JSON.

### Bootstrap

```bash
cat agent.py | ssh -o BatchMode=yes user@host "python3 -u -"
```

Agent sends ready message:
```json
{"ok": true, "v": 1}
```

### Message Format

**Request** (client → agent):
```json
{"id": 1, "m": "read", "p": {"path": "/file.rs"}}
```

**Response** (agent → client) - three types:
```json
{"id": 1, "d": {...}}        // data: intermediate streaming data (0 or more)
{"id": 1, "r": {...}}        // result: success, request complete
{"id": 1, "e": "message"}    // error: failure, request complete
```

A request receives: `d*` then (`r` | `e`) — zero or more data messages, then exactly one result or error.

### Multiplexing

Multiple requests in-flight, responses arrive in any order:

```
→ {"id": 1, "m": "read", "p": {"path": "/big.rs"}}
→ {"id": 2, "m": "stat", "p": {"path": "/small.rs"}}
← {"id": 2, "r": {"size": 100, "mtime": 1706000000}}
← {"id": 1, "r": {"data": "base64..."}}
```

### Streaming

Any method can stream by sending multiple `d` messages before final `r`:

**Large file read:**
```
→ {"id": 1, "m": "read", "p": {"path": "/huge.bin"}}
← {"id": 1, "d": {"data": "base64chunk1..."}}
← {"id": 1, "d": {"data": "base64chunk2..."}}
← {"id": 1, "r": {"size": 131072}}
```

**Process execution with live output:**
```
→ {"id": 1, "m": "exec", "p": {"cmd": "rg", "args": ["TODO", "."]}}
← {"id": 1, "d": {"out": "src/main.rs:10:// TODO fix"}}
← {"id": 1, "d": {"out": "src/lib.rs:20:// TODO test"}}
← {"id": 1, "d": {"err": "some warning"}}
← {"id": 1, "r": {"code": 0}}
```

### Cancellation

Send cancel, agent terminates the operation:

```
→ {"id": 1, "m": "exec", "p": {"cmd": "sleep", "args": ["100"]}}
→ {"id": 0, "m": "cancel", "p": {"id": 1}}
← {"id": 0, "r": {}}
← {"id": 1, "e": "cancelled"}
```

### Methods

**File Operations:**

| Method | Params | Streams | Result |
|--------|--------|---------|--------|
| `read` | `path`, `off?`, `len?` | `{data}` chunks for large files | `{size}` |
| `write` | `path`, `data` | — | `{size}` |
| `stat` | `path`, `link?` | — | `{size, mtime, mode, uid, gid, dir, file, link}` |
| `ls` | `path` | — | `{entries: [{name, path, dir, file, link, size, mtime, mode}]}` |
| `rm` | `path` | — | `{}` |
| `rmdir` | `path` | — | `{}` |
| `mkdir` | `path`, `parents?` | — | `{}` |
| `mv` | `from`, `to` | — | `{}` |
| `cp` | `from`, `to` | — | `{size}` |
| `realpath` | `path` | — | `{path}` |
| `chmod` | `path`, `mode` | — | `{}` |

**Process Operations:**

| Method | Params | Streams | Result |
|--------|--------|---------|--------|
| `exec` | `cmd`, `args`, `cwd?` | `{out?, err?}` live output | `{code}` |
| `kill` | `id` | — | `{}` |
| `cancel` | `id` | — | `{}` |

### DirEntry Format (in `ls` result)

```json
{
  "name": "file.rs",
  "path": "/home/user/project/file.rs",
  "dir": false,
  "file": true,
  "link": false,
  "size": 1234,
  "mtime": 1706300000,
  "mode": 33188
}
```

### Binary Data

All binary data is base64-encoded. Simple, safe over any channel, and SSH compression negates most overhead.

```json
{"id": 1, "d": {"data": "SGVsbG8gV29ybGQ="}}
```

### Design Rationale

1. **Single streaming model**: No separate "background" vs "blocking" — `exec` always streams, client can cancel anytime
2. **No subscriptions**: Streaming is implicit, not opt-in. Simpler agent code.
3. **Short keys**: `m`, `p`, `d`, `r`, `e` to reduce bandwidth over SSH
4. **Everything has an ID**: No special "notification" concept. All messages tied to a request.
5. **Stateless cancellation**: `cancel` works on any in-flight request, agent cleans up
6. **Base64 everywhere**: One encoding, works over any channel, SSH compresses anyway

## Implementation Details

### RemoteFileSystem

**New file**: `crates/fresh-editor/src/model/remote_filesystem.rs`

```rust
pub struct RemoteFileSystem {
    channel: Arc<AgentChannel>,
    connection_string: String,
    metadata_cache: Arc<RwLock<LruCache<PathBuf, CachedMetadata>>>,
    state: Arc<RwLock<ConnectionState>>,
}

impl FileSystem for RemoteFileSystem {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        let response = self.channel.request_blocking(AgentRequest::ReadFile {
            path: path.to_string_lossy().to_string(),
            offset: None,
            length: None,
        })?;

        base64::decode(&response.data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        self.channel.request_blocking(AgentRequest::WriteFile {
            path: path.to_string_lossy().to_string(),
            data: base64::encode(data),
            atomic: true,
        })?;

        // Invalidate cache
        self.invalidate_cache(path);
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let response = self.channel.request_blocking(AgentRequest::ReadDir {
            path: path.to_string_lossy().to_string(),
        })?;

        // Convert and cache entries
        response.entries.into_iter()
            .map(|e| self.convert_entry(e))
            .collect()
    }

    // ... implement all FileSystem trait methods
}
```

### ProcessSpawner Trait

**New file**: `crates/fresh-editor/src/services/process_spawner.rs`

```rust
#[async_trait]
pub trait ProcessSpawner: Send + Sync {
    async fn spawn(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Result<SpawnResult, SpawnError>;
}

pub struct LocalProcessSpawner;

#[async_trait]
impl ProcessSpawner for LocalProcessSpawner {
    async fn spawn(&self, command: String, args: Vec<String>, cwd: Option<String>) -> Result<SpawnResult, SpawnError> {
        let output = tokio::process::Command::new(&command)
            .args(&args)
            .current_dir(cwd.as_deref().unwrap_or("."))
            .output()
            .await?;

        Ok(SpawnResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

pub struct RemoteProcessSpawner {
    channel: Arc<AgentChannel>,
}

#[async_trait]
impl ProcessSpawner for RemoteProcessSpawner {
    async fn spawn(&self, command: String, args: Vec<String>, cwd: Option<String>) -> Result<SpawnResult, SpawnError> {
        let response = self.channel.request(AgentRequest::SpawnProcess {
            command, args, cwd, timeout: Some(300),
        }).await?;

        Ok(SpawnResult {
            stdout: String::from_utf8_lossy(&base64::decode(&response.stdout)?).to_string(),
            stderr: String::from_utf8_lossy(&base64::decode(&response.stderr)?).to_string(),
            exit_code: response.exit_code,
        })
    }
}
```

### SSH Connection Manager

**New file**: `crates/fresh-editor/src/services/ssh/mod.rs`

```rust
pub struct SshConnection {
    process: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    params: ConnectionParams,
}

pub struct ConnectionParams {
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
}

impl SshConnection {
    pub async fn connect(params: ConnectionParams) -> Result<Self, SshError> {
        let agent_code = include_str!("../resources/remote_agent.py");

        let mut cmd = tokio::process::Command::new("ssh");
        cmd.arg("-o").arg("BatchMode=yes");

        if let Some(port) = params.port {
            cmd.arg("-p").arg(port.to_string());
        }
        if let Some(ref identity) = params.identity_file {
            cmd.arg("-i").arg(identity);
        }

        cmd.arg(format!("{}@{}", params.user, params.host));
        cmd.arg("python3").arg("-u").arg("-");

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        // Bootstrap agent
        let stdin = child.stdin.take().unwrap();
        stdin.write_all(agent_code.as_bytes()).await?;
        stdin.shutdown().await?; // Signal EOF for agent code

        // Wait for ready message
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let ready_line = stdout.lines().next_line().await?
            .ok_or(SshError::AgentNotReady)?;

        let ready: ReadyMessage = serde_json::from_str(&ready_line)?;
        if !ready.ready {
            return Err(SshError::AgentNotReady);
        }

        Ok(Self { process: child, stdin, stdout, params })
    }
}
```

### AgentChannel (Multiplexer)

Handles streaming responses with a simple callback model:

```rust
pub struct AgentChannel {
    request_tx: mpsc::Sender<String>,
    pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
    next_id: AtomicU64,
}

struct PendingRequest {
    data_tx: mpsc::Sender<Value>,      // For streaming data
    result_tx: oneshot::Sender<Result<Value, String>>,  // Final result/error
}

impl AgentChannel {
    /// Non-streaming request: collect all data, return final result
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, SshError> {
        let (mut data_rx, result_rx) = self.request_streaming(method, params).await?;

        // Drain and ignore intermediate data for non-streaming callers
        while data_rx.recv().await.is_some() {}

        result_rx.await?
    }

    /// Streaming request: returns channel for data + future for final result
    pub async fn request_streaming(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(mpsc::Receiver<Value>, oneshot::Receiver<Result<Value, String>>), SshError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (data_tx, data_rx) = mpsc::channel(64);
        let (result_tx, result_rx) = oneshot::channel();

        self.pending.lock().await.insert(id, PendingRequest { data_tx, result_tx });

        let msg = json!({"id": id, "m": method, "p": params});
        self.request_tx.send(msg.to_string()).await?;

        Ok((data_rx, result_rx))
    }

    /// Background read loop - routes responses to pending requests
    async fn read_loop(
        mut stdout: BufReader<ChildStdout>,
        pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
    ) {
        let mut line = String::new();
        while stdout.read_line(&mut line).await.is_ok() {
            if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                let id = msg["id"].as_u64().unwrap_or(0);

                let mut pending = pending.lock().await;
                if let Some(req) = pending.get(&id) {
                    if let Some(d) = msg.get("d") {
                        // Streaming data - send to channel
                        let _ = req.data_tx.send(d.clone()).await;
                    } else if let Some(r) = msg.get("r") {
                        // Success - complete request
                        if let Some(req) = pending.remove(&id) {
                            let _ = req.result_tx.send(Ok(r.clone()));
                        }
                    } else if let Some(e) = msg.get("e") {
                        // Error - complete request
                        if let Some(req) = pending.remove(&id) {
                            let _ = req.result_tx.send(Err(e.as_str().unwrap_or("unknown").to_string()));
                        }
                    }
                }
            }
            line.clear();
        }
    }

    pub fn request_blocking(&self, method: &str, params: Value) -> io::Result<Value> {
        tokio::runtime::Handle::current()
            .block_on(self.request(method, params))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
```

### CLI Parsing Changes

**Modify**: `crates/fresh-editor/src/main.rs`

```rust
enum ParsedLocation {
    Local(FileLocation),
    Remote {
        user: String,
        host: String,
        port: Option<u16>,
        path: String,
        line: Option<usize>,
        column: Option<usize>,
    },
}

fn parse_location(input: &str) -> ParsedLocation {
    // Pattern: user@host:path or user@host:port:path
    if let Some(at_pos) = input.find('@') {
        let user = &input[..at_pos];
        let rest = &input[at_pos + 1..];

        // Find first colon (host:path separator)
        if let Some(colon_pos) = rest.find(':') {
            let host = &rest[..colon_pos];
            let after_host = &rest[colon_pos + 1..];

            // Check if next segment is port (all digits) followed by colon
            let (port, path_part) = if let Some(next_colon) = after_host.find(':') {
                let maybe_port = &after_host[..next_colon];
                if maybe_port.chars().all(|c| c.is_ascii_digit()) {
                    (Some(maybe_port.parse().unwrap()), &after_host[next_colon + 1..])
                } else {
                    (None, after_host)
                }
            } else {
                (None, after_host)
            };

            // Parse line:col from path
            let (path, line, column) = parse_path_line_col(path_part);

            if !user.is_empty() && !host.is_empty() && !path.is_empty() {
                return ParsedLocation::Remote {
                    user: user.to_string(),
                    host: host.to_string(),
                    port,
                    path,
                    line,
                    column,
                };
            }
        }
    }

    ParsedLocation::Local(parse_file_location(input))
}
```

### Editor Integration

**Modify**: `crates/fresh-editor/src/app/mod.rs`

Add to Editor struct:
```rust
pub struct Editor {
    // ... existing fields
    filesystem: Arc<dyn FileSystem + Send + Sync>,
    process_spawner: Arc<dyn ProcessSpawner + Send + Sync>,
    remote_connection: Option<Arc<SshConnection>>,
}
```

Modify SpawnProcess handler (~line 4514):
```rust
PluginCommand::SpawnProcess { command, args, cwd, callback_id } => {
    let spawner = self.process_spawner.clone();
    let sender = bridge.sender();

    runtime.spawn(async move {
        let effective_cwd = cwd.unwrap_or_else(|| /* default */);

        match spawner.spawn(command, args, Some(effective_cwd)).await {
            Ok(result) => {
                let _ = sender.send(AsyncMessage::PluginProcessOutput {
                    process_id: callback_id.as_u64(),
                    stdout: result.stdout,
                    stderr: result.stderr,
                    exit_code: result.exit_code,
                });
            }
            Err(e) => {
                let _ = sender.send(AsyncMessage::PluginProcessOutput {
                    process_id: callback_id.as_u64(),
                    stdout: String::new(),
                    stderr: e.to_string(),
                    exit_code: -1,
                });
            }
        }
    });
}
```

## Python Agent

**New file**: `crates/fresh-editor/resources/remote_agent.py`

Simplified agent with streaming support (~150 lines):

```python
#!/usr/bin/env python3
"""Fresh Remote Agent"""
import sys, os, json, base64, stat, shutil, subprocess, threading, select

CHUNK = 65536
procs = {}  # id -> Popen
cancel = set()  # cancelled request ids
lock = threading.Lock()

def send(id, **kw):
    sys.stdout.write(json.dumps({"id": id, **kw}) + "\n")
    sys.stdout.flush()

def path(p):
    """Validate and canonicalize path"""
    if not p: raise ValueError("empty path")
    return os.path.realpath(os.path.expanduser(p))

def b64(data): return base64.b64encode(data).decode()
def unb64(s): return base64.b64decode(s)

# File operations
def cmd_read(id, p, path, off=0, len=None):
    f = open(path(p["path"]), "rb")
    if off: f.seek(off)
    size = 0
    while True:
        chunk = f.read(len or CHUNK)
        if not chunk: break
        size += len(chunk)
        send(id, d={"data": b64(chunk)})
        if len: break  # specific length requested
    f.close()
    send(id, r={"size": size})

def cmd_write(id, p, path):
    pth = path(p["path"])
    data = unb64(p["data"])
    tmp = f"{pth}.fresh-{os.getpid()}"
    mode = os.stat(pth).st_mode if os.path.exists(pth) else None
    with open(tmp, "wb") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    if mode: os.chmod(tmp, mode)
    os.rename(tmp, pth)
    send(id, r={"size": len(data)})

def cmd_stat(id, p, path):
    pth = path(p["path"])
    follow = p.get("link", True)
    st = os.stat(pth, follow_symlinks=follow)
    send(id, r={
        "size": st.st_size, "mtime": int(st.st_mtime), "mode": st.st_mode,
        "uid": st.st_uid, "gid": st.st_gid,
        "dir": stat.S_ISDIR(st.st_mode), "file": stat.S_ISREG(st.st_mode),
        "link": stat.S_ISLNK(os.lstat(pth).st_mode) if follow else False,
    })

def cmd_ls(id, p, path):
    pth = path(p["path"])
    entries = []
    for e in os.scandir(pth):
        try:
            st = e.stat(follow_symlinks=False)
            entries.append({
                "name": e.name, "path": os.path.join(pth, e.name),
                "dir": e.is_dir(), "file": e.is_file(), "link": e.is_symlink(),
                "size": st.st_size, "mtime": int(st.st_mtime), "mode": st.st_mode,
            })
        except OSError: pass
    send(id, r={"entries": entries})

def cmd_rm(id, p, path): os.unlink(path(p["path"])); send(id, r={})
def cmd_rmdir(id, p, path): os.rmdir(path(p["path"])); send(id, r={})
def cmd_mkdir(id, p, path):
    pth = path(p["path"])
    (os.makedirs if p.get("parents") else os.mkdir)(pth, exist_ok=True)
    send(id, r={})
def cmd_mv(id, p, path): os.rename(path(p["from"]), path(p["to"])); send(id, r={})
def cmd_cp(id, p, path):
    shutil.copy2(path(p["from"]), path(p["to"]))
    send(id, r={"size": os.path.getsize(path(p["to"]))})
def cmd_realpath(id, p, path): send(id, r={"path": path(p["path"])})
def cmd_chmod(id, p, path): os.chmod(path(p["path"]), p["mode"]); send(id, r={})

# Process operations - streaming
def cmd_exec(id, p, path):
    cwd = path(p["cwd"]) if p.get("cwd") else None
    try:
        proc = subprocess.Popen(
            [p["cmd"]] + p.get("args", []), cwd=cwd,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
    except FileNotFoundError:
        send(id, e=f"command not found: {p['cmd']}")
        return

    with lock: procs[id] = proc

    def stream():
        while proc.poll() is None:
            if id in cancel:
                proc.terminate()
                send(id, e="cancelled")
                return
            # Non-blocking read
            rlist, _, _ = select.select([proc.stdout, proc.stderr], [], [], 0.05)
            for fd in rlist:
                data = fd.read(4096)
                if data:
                    key = "out" if fd == proc.stdout else "err"
                    send(id, d={key: b64(data)})
        # Drain remaining
        out, err = proc.communicate()
        if out: send(id, d={"out": b64(out)})
        if err: send(id, d={"err": b64(err)})
        send(id, r={"code": proc.returncode})
        with lock: procs.pop(id, None)

    threading.Thread(target=stream, daemon=True).start()

def cmd_kill(id, p, path):
    with lock:
        proc = procs.get(p["id"])
    if proc:
        proc.terminate()
        send(id, r={})
    else:
        send(id, e="no such process")

def cmd_cancel(id, p, path):
    target = p["id"]
    cancel.add(target)
    with lock:
        proc = procs.get(target)
    if proc: proc.terminate()
    send(id, r={})

METHODS = {
    "read": cmd_read, "write": cmd_write, "stat": cmd_stat, "ls": cmd_ls,
    "rm": cmd_rm, "rmdir": cmd_rmdir, "mkdir": cmd_mkdir, "mv": cmd_mv,
    "cp": cmd_cp, "realpath": cmd_realpath, "chmod": cmd_chmod,
    "exec": cmd_exec, "kill": cmd_kill, "cancel": cmd_cancel,
}

def main():
    send(0, ok=True, v=1)
    for line in sys.stdin:
        if not line.strip(): continue
        try:
            req = json.loads(line)
            id, m, p = req["id"], req["m"], req.get("p", {})
            if m not in METHODS:
                send(id, e=f"unknown method: {m}")
            else:
                METHODS[m](id, p, path)
        except json.JSONDecodeError as e:
            send(0, e=f"parse error: {e}")
        except Exception as e:
            send(req.get("id", 0), e=str(e))

if __name__ == "__main__":
    main()
```

## Caching Strategy

### Metadata Cache
- LRU cache with 1000 entries
- 5-second TTL for cached metadata
- Invalidated on write operations

### Directory Cache
- Cache read_dir results for 2 seconds
- Invalidated when files are created/deleted in that directory

### Write-Through
- All writes invalidate relevant cache entries
- Parent directory cache also invalidated

## Error Handling

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Agent not responding")]
    AgentTimeout,

    #[error("Agent protocol error: {0}")]
    ProtocolError(String),

    #[error("SSH process terminated: {0}")]
    ProcessDied(String),

    #[error("Reconnection failed after {0} attempts")]
    ReconnectionFailed(u32),
}
```

### Recovery Strategy

1. **Transient errors**: Retry with exponential backoff (max 3 attempts)
2. **Connection loss**:
   - Queue pending operations
   - Attempt reconnection (max 5 attempts over 30 seconds)
   - Replay queued operations on success
   - Show error dialog after all retries fail
3. **Agent crash**: Restart agent, invalidate all caches

## UX Integration

### Status Bar

Normal: `[SSH: user@host] file.rs [+]`

Disconnected: `[SSH: RECONNECTING...] file.rs [+]`

With latency: `[SSH: user@host (120ms)] file.rs [+]`

### Open Remote Command

Add action in `keybindings.rs`:
```rust
Action::OpenRemote  // Opens connection string prompt
```

Add command in `commands.rs`:
```rust
Command {
    name: "Open Remote",
    description: "Open a file or directory on a remote server via SSH",
    action: Action::OpenRemote,
    contexts: vec![],
    custom_contexts: vec![],
    source: CommandSource::Builtin,
}
```

## Security Considerations

1. **Path Traversal**: Agent canonicalizes all paths via `os.path.realpath()`
2. **Command Injection**: Process args passed as array, not shell string
3. **SSH Auth**: Delegated to system SSH client (no password storage)
4. **Agent Integrity**: Embedded in binary, transferred at connection time

## Files to Create/Modify

### New Files
- `crates/fresh-editor/src/model/remote_filesystem.rs` - RemoteFileSystem impl
- `crates/fresh-editor/src/services/ssh/mod.rs` - SSH connection management
- `crates/fresh-editor/src/services/ssh/channel.rs` - AgentChannel multiplexer
- `crates/fresh-editor/src/services/process_spawner.rs` - ProcessSpawner trait
- `crates/fresh-editor/resources/remote_agent.py` - Python agent

### Modified Files
- `crates/fresh-editor/src/main.rs` - CLI parsing for remote locations
- `crates/fresh-editor/src/app/mod.rs` - Editor struct, SpawnProcess handler
- `crates/fresh-editor/src/keybindings.rs` - Add Action::OpenRemote
- `crates/fresh-editor/src/commands.rs` - Add "Open Remote" command

## Testing Strategy

### Unit Tests
- Agent command handlers (mock filesystem)
- RemoteFileSystem (mock AgentChannel)
- Connection string parsing

### Integration Tests
- Local agent spawn and protocol test
- Full round-trip file operations

### Manual Testing
- SSH to real server
- Edit files, verify save
- Test git_grep, live_grep plugins
- Simulate disconnect/reconnect
- Large file operations

## Implementation Phases

### Phase 1: Core Infrastructure
- Python agent implementation
- SSH connection management
- AgentChannel multiplexing
- Basic RemoteFileSystem (read_file, write_file, read_dir, metadata)

### Phase 2: Full FileSystem
- Implement remaining FileSystem methods
- Caching layer
- Error handling and retries

### Phase 3: Process Spawning
- ProcessSpawner trait abstraction
- RemoteProcessSpawner implementation
- Modify SpawnProcess handler
- Test with plugins

### Phase 4: CLI Integration
- Connection string parsing
- Editor initialization with remote filesystem
- "Open Remote" command
- Status bar integration

### Phase 5: Polish
- Reconnection logic
- Large file streaming
- Performance tuning
- Documentation
