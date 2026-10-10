//! The execution core: device link, helper and Chrome connections, the
//! at-most-once journal, and the structured read/act entry points that both
//! the legacy JSON surface and the agent command layer (`agent.rs`) use.
use crate::{
    agent::AgentState,
    api::{BrowserPlan, BrowserRead, Code, Error, Plan, Read, Receipt, Result, Scene, VisualOp, VisualPlan, VisualRead, MAX_INLINE},
    artifact::{valid, Artifacts, MAX_ARTIFACT},
    bridge::{Bridge, Observation},
    browser::Browser,
    device::{Adb, Device, Paths},
    format,
    journal::{unknown, Journal},
    visual,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub use crate::agent::{ModelImage, ModelResponse};

pub struct Engine {
    paths: Paths,
    adb: Option<Adb>,
    device: Option<Device>,
    bridge: Option<Bridge>,
    browser: Option<Browser>,
    pub(crate) scene: Option<Scene>,
    pub(crate) artifacts: Artifacts,
    journal: Journal,
    pub(crate) agent: AgentState,
    /// Random per-process salt for operation ids. Request identities such as
    /// JSON-RPC ids restart in every client session, while the journal is
    /// durable; without the salt a new session could collide with old records.
    session: String,
    request_counter: u64,
}

/// Transport failures worth one reconnect-and-retry for read-only calls.
fn reconnectable(code: Code) -> bool {
    matches!(code, Code::Auth | Code::Device | Code::Helper | Code::Io | Code::Sequence | Code::Timeout | Code::Protocol)
}

impl Engine {
    /// Opens local state only. The adb executable and the enrolled device are
    /// resolved on first use, so a server can start (and explain problems in
    /// its tool results) before a device is connected.
    pub fn open() -> Result<Self> {
        Self::at(Paths::discover()?)
    }

    pub(crate) fn at(paths: Paths) -> Result<Self> {
        Ok(Self {
            artifacts: Artifacts::new(paths.clone()),
            journal: Journal::open(&paths)?,
            paths,
            adb: None,
            device: None,
            bridge: None,
            browser: None,
            scene: None,
            agent: AgentState::default(),
            session: session_id(),
            request_counter: 0,
        })
    }

    /// Attaches a pre-connected helper bridge (a fake one in tests).
    #[cfg(test)]
    pub(crate) fn attach(&mut self, bridge: Bridge, device: Device) {
        self.adb = Some(Adb::for_tests());
        self.device = Some(device);
        self.bridge = Some(bridge);
    }

    pub fn root(&self) -> &Path {
        &self.paths.root
    }

    fn link(&mut self) -> Result<(Adb, Device)> {
        let adb = match &self.adb {
            Some(adb) => adb.clone(),
            None => self.adb.insert(Adb::discover()?).clone(),
        };
        let device = match &self.device {
            Some(device) => device.clone(),
            None => self.device.insert(adb.resolve(&self.paths)?).clone(),
        };
        Ok((adb, device))
    }

    pub(crate) fn bridge(&mut self) -> Result<&mut Bridge> {
        let bridge = match self.bridge.take() {
            Some(bridge) => bridge,
            None => {
                let (adb, device) = self.link()?;
                Bridge::connect(adb, device)?
            }
        };
        Ok(self.bridge.insert(bridge))
    }

    pub(crate) fn browser(&mut self) -> Result<&mut Browser> {
        let browser = match self.browser.take() {
            Some(browser) => browser,
            None => {
                let (adb, device) = self.link()?;
                Browser::connect(adb, device)?
            }
        };
        Ok(self.browser.insert(browser))
    }

    fn reset_connections(&mut self) {
        self.bridge = None;
        self.browser = None;
        self.device = None;
    }

    /// Runs a read-only helper call, reconnecting once on transport failure.
    pub(crate) fn bridge_read<T>(&mut self, mut f: impl FnMut(&mut Bridge) -> Result<T>) -> Result<T> {
        match self.bridge().and_then(&mut f) {
            Err(error) if reconnectable(error.code) => {
                self.reset_connections();
                f(self.bridge()?)
            }
            result => result,
        }
    }

    /// Runs a read-only Chrome call, reconnecting once on transport failure.
    pub(crate) fn browser_read<T>(&mut self, mut f: impl FnMut(&mut Browser) -> Result<T>) -> Result<T> {
        match self.browser().and_then(&mut f) {
            Err(error) if reconnectable(error.code) => {
                self.reset_connections();
                f(self.browser()?)
            }
            result => result,
        }
    }

    pub fn read(&mut self, r: Read) -> Result<Value> {
        match r {
            Read::Status => {
                let (g, cap) = self.bridge_read(|bridge| bridge.status())?;
                Ok(json!({"ok":1,"g":g,"cap":cap}))
            }
            Read::Observe { base, detail } => {
                let scene = match self.bridge_read(|bridge| bridge.observe(base.as_deref(), detail))? {
                    Observation::Unchanged(g) => return Ok(json!({"=":1,"o":base.unwrap_or_else(||g.to_string().into()),"g":g})),
                    Observation::Scene(scene) => scene,
                };
                let out = if detail == 0 {
                    scene.json()
                } else {
                    let bytes = serde_json::to_vec(&scene.json()).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
                    json!({"o":scene.observation,"g":scene.generation,"artifact":self.artifacts.put(&bytes)?})
                };
                self.scene = Some(scene);
                Ok(out)
            }
            Read::Artifact { id, range } if id.starts_with('h') => self.artifacts.read(&id, range),
            Read::Artifact { id, range } => {
                if !valid(&id) {
                    return Err(Error::new(Code::Artifact, "invalid artifact id"));
                }
                if range.is_some_and(|r| r.end.saturating_sub(r.start) > MAX_INLINE as u64) {
                    return Err(Error::new(Code::Bounds, "artifact range exceeds 2800 bytes"));
                }
                let (size, start, bytes) = self.bridge_read(|bridge| bridge.artifact(&id, range))?;
                let end = start + bytes.len() as u64;
                Ok(json!({"id":id,"size":size,"start":start,"data":STANDARD.encode(&bytes),"more":(end<size) as u8}))
            }
            Read::Browser { op } => self.browser_read(|browser| browser.read(op.clone())),
            Read::Capabilities => self.bridge_read(|bridge| bridge.query("capabilities", Value::Null)),
            Read::Location => self.bridge_read(|bridge| bridge.query("location", Value::Null)),
            Read::Notifications => self.bridge_read(|bridge| bridge.query("notifications", Value::Null)),
            Read::Visual(VisualRead::Hash(id)) => visual::hash(&self.visual_bytes(&id)?),
            Read::Visual(VisualRead::Diff(a, b)) => visual::diff(&self.visual_bytes(&a)?, &self.visual_bytes(&b)?),
        }
    }

    /// The current Android scene, reusing the cached one when the helper
    /// reports that nothing changed since it was observed.
    pub(crate) fn ensure_scene(&mut self) -> Result<Scene> {
        let base = self.scene.as_ref().map(|scene| scene.observation.clone());
        let scene = match self.bridge_read(|bridge| bridge.observe(base.as_deref(), 0))? {
            Observation::Unchanged(generation) => {
                self.scene.clone().filter(|scene| scene.generation == generation).ok_or_else(|| Error::new(Code::Stale, "the Android scene changed; retry the command"))?
            }
            Observation::Scene(scene) => scene,
        };
        self.scene = Some(scene.clone());
        Ok(scene)
    }

    pub(crate) fn read_page(&mut self) -> Result<Value> {
        self.read(Read::Browser { op: BrowserRead::Observe })
    }

    /// Called once the agent has read the device: outcomes left pending by an
    /// earlier process become `unknown`, which lifts the mutation fence.
    pub(crate) fn reconcile_outcomes(&mut self) -> Result<()> {
        self.journal.reconcile().map(|_| ())
    }

    pub fn act(&mut self, p: Plan) -> Result<Value> {
        // The helper drops idle sessions; probe (and reconnect) before the
        // journal records anything, so a dead socket is an error, not `unknown`.
        self.bridge_read(|bridge| bridge.status())?;
        to_value(self.run_plan(&p)?)
    }

    /// Runs a generation-checked Android plan at most once per id.
    ///
    /// Connection problems are reported as errors before anything is
    /// journaled. Once the pending record is written, any transport failure
    /// becomes `unknown`: the plan may have reached the device.
    pub(crate) fn run_plan(&mut self, p: &Plan) -> Result<Receipt> {
        let digest = self.digest(&p.wire(0), true)?;
        self.bridge()?;
        if let Some(prior) = self.journal.begin(&p.id, p.generation, &digest)? {
            return Ok(prior);
        }
        let receipt = match self.bridge().and_then(|bridge| bridge.act(p)) {
            Ok(receipt) => receipt,
            Err(_) => {
                // The plan may have reached the device. Record that as uncertain,
                // so a retry is answered from the journal instead of dispatched again.
                self.bridge = None;
                let lost = unknown(&p.id, p.generation);
                let _ = self.journal.finish(&digest, &lost);
                return Ok(lost);
            }
        };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(unknown(&p.id, receipt.g));
        }
        if receipt.m > 0 {
            self.scene = None;
        }
        Ok(receipt)
    }

    pub fn browser_act(&mut self, p: BrowserPlan) -> Result<Value> {
        self.browser_read(|browser| browser.read(BrowserRead::Tabs))?;
        to_value(self.run_browser_plan(&p, false)?)
    }

    /// Runs a Chrome plan at most once per id. `prepared` plans were compiled
    /// against an observation taken moments ago, so the tab list is not
    /// re-synchronized before running them.
    pub(crate) fn run_browser_plan(&mut self, p: &BrowserPlan, prepared: bool) -> Result<Receipt> {
        let digest = self.digest(&p.wire(0), true)?;
        self.browser()?;
        if let Some(prior) = self.journal.begin(&p.id, p.generation, &digest)? {
            return Ok(prior);
        }
        let outcome = match self.browser().and_then(|browser| if prepared { browser.act_prepared(p) } else { browser.act(p) }) {
            Ok(outcome) => outcome,
            Err(_) => {
                self.browser = None;
                let lost = unknown(&p.id, p.generation);
                let _ = self.journal.finish(&digest, &lost);
                return Ok(lost);
            }
        };
        // Chrome has already acted: a failed artifact write must not abandon
        // the journal record or turn the outcome into an error.
        let artifact = outcome.artifact.as_deref().and_then(|bytes| self.artifacts.put(bytes).ok());
        let receipt = Receipt {
            id: p.id.clone(),
            ok: u8::from(outcome.error.is_none()),
            g: outcome.generation,
            m: outcome.mutations,
            at: outcome.at,
            e: outcome.error.map(Into::into),
            partial: outcome.partial.then_some(1),
            next: (outcome.error == Some("unknown")).then(|| "observe".into()),
            artifact,
        };
        if self.journal.finish(&digest, &receipt).is_err() {
            return Ok(unknown(&p.id, receipt.g));
        }
        Ok(receipt)
    }

    pub fn visual_act(&mut self, p: VisualPlan) -> Result<Value> {
        // Cropping is local and repeatable, so do the fallible work before
        // journaling; a failure then leaves no pending record behind.
        let digest = self.digest(&p.wire(0), false)?;
        let VisualOp::Crop { artifact, x, y, w, h } = &p.op;
        let source = self.visual_bytes(artifact)?;
        let cropped = self.artifacts.put(&visual::crop(&source, *x, *y, *w, *h)?)?;
        if let Some(prior) = self.journal.begin(&p.id, p.generation, &digest)? {
            return to_value(prior);
        }
        let receipt = Receipt { id: p.id.clone(), ok: 1, g: p.generation, m: 1, at: None, e: None, partial: None, next: None, artifact: Some(cropped) };
        if self.journal.finish(&digest, &receipt).is_err() {
            return to_value(unknown(&p.id, p.generation));
        }
        to_value(receipt)
    }

    /// Plans are bound to the enrolled hardware so a replayed id can never
    /// match a record made for a different device.
    fn digest(&mut self, wire: &Value, device_bound: bool) -> Result<String> {
        let mut hash = Sha256::new();
        if device_bound {
            hash.update(self.link()?.1.hardware.as_bytes());
        }
        hash.update(serde_json::to_vec(wire).map_err(|e| Error::new(Code::Protocol, e.to_string()))?);
        Ok(hex(&hash.finalize()))
    }

    pub(crate) fn download(&mut self, id: &str) -> Result<Vec<u8>> {
        if !valid(id) {
            return Err(Error::new(Code::Artifact, "invalid artifact id"));
        }
        self.bridge_read(|bridge| bridge.download(id, MAX_ARTIFACT))
    }

    fn visual_bytes(&mut self, id: &str) -> Result<Vec<u8>> {
        if id.starts_with('h') {
            self.artifacts.bytes(id)
        } else {
            self.download(id)
        }
    }

    pub(crate) fn operation_id(&mut self, request_identity: Option<&str>, kind: &str) -> String {
        self.request_counter = self.request_counter.wrapping_add(1);
        let identity = request_identity.map_or_else(|| format!("local-{}", self.request_counter), str::to_owned);
        operation_id(&self.session, kind, &identity)
    }

    /// Saves media for the user or a shell agent and returns its path.
    pub fn export(&self, bytes: &[u8]) -> Result<PathBuf> {
        self.artifacts.export(bytes, format::extension(format::mime_type(bytes)))
    }
}

fn operation_id(session: &str, kind: &str, identity: &str) -> String {
    let mut hash = Sha256::new();
    for part in [session, kind, identity] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("n{}", &hex(&hash.finalize())[..31])
}

fn session_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() ^ (u128::from(std::process::id()) << 64);
        bytes = fallback.to_le_bytes();
    }
    hex(&bytes)
}

fn to_value(receipt: Receipt) -> Result<Value> {
    serde_json::to_value(receipt).map_err(|e| Error::new(Code::Protocol, e.to_string()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn plain_error(error: &Error) -> String {
    format::plain_error_model(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_valid_and_scoped_to_one_process() {
        let first = operation_id("session-a", "android", "7#group-0");
        assert!(valid(&first) && first.len() == 32);
        assert_eq!(first, operation_id("session-a", "android", "7#group-0"));
        assert_ne!(first, operation_id("session-b", "android", "7#group-0"), "a restarted client reusing JSON-RPC id 7 must not collide");
        assert_ne!(first, operation_id("session-a", "browser", "7#group-0"));
        assert_ne!(session_id(), session_id());
    }

    #[test]
    fn engine_opens_without_a_device_or_adb() {
        let d = tempfile::tempdir().unwrap();
        let mut engine = Engine::at(Paths::at(d.path().to_path_buf()).unwrap()).unwrap();
        let a = engine.operation_id(Some("1"), "android");
        let b = engine.operation_id(None, "android");
        assert_ne!(a, b);
        assert!(engine.export(b"\x89PNG....").unwrap().extension().is_some_and(|ext| ext == "png"));
    }
}
