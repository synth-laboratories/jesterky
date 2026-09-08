//! Durable, conservative pre-request reservation for text-only annotation runs.
//! Reserved maxima are never refunded: retries and uncertain network outcomes
//! cannot spend the same allowance twice. This is not a billing estimate.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs::{self, OpenOptions}, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct BudgetConfig {
    pub max_cost_usd: f64,
    pub input_usd_per_million: f64,
    pub output_usd_per_million: f64,
    pub max_output_tokens: u64,
    pub max_request_bytes: u64,
    #[serde(default)]
    pub max_total_tokens: Option<u64>,
    pub ledger_path: PathBuf,
}
#[derive(Deserialize, Serialize)]
struct Ledger { config: BudgetConfig, reserved_micros: u64, requests: u64, #[serde(default)] reserved_tokens: u64 }
pub(crate) struct Budget(BudgetConfig);
impl Budget {
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var("JESTERKY_PROXY_BUDGET_JSON") {
            Ok(raw) => Self::new(serde_json::from_str(&raw).map_err(|e|format!("invalid proxy budget: {e}"))?).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn new(config: BudgetConfig) -> Result<Self, String> {
        if [config.max_cost_usd,config.input_usd_per_million,config.output_usd_per_million].iter().any(|v|!v.is_finite() || *v<=0.)
            || config.max_total_tokens == Some(0) || config.max_cost_usd>10000. || config.max_output_tokens==0 || config.max_output_tokens>32768
            || config.max_request_bytes==0 || config.max_request_bytes>1024*1024 || !config.ledger_path.is_absolute() {
            return Err("proxy budget requires finite positive prices/cap, bounded tokens/body, and an absolute ledger path".into());
        }
        Ok(Self(config))
    }
    pub fn reserve(&self, payload:&mut Value) -> Result<(),String> {
        let c=&self.0;
        // Token ceilings apply to text, including serialized tool definitions.
        fn media(v:&Value)->bool { match v {Value::Object(o)=>o.keys().any(|k|matches!(k.as_str(),"image_url"|"input_audio"|"audio"|"video_url"))||o.values().any(media),Value::Array(a)=>a.iter().any(media),_=>false} }
        if media(&payload["messages"]) { return Err("budgeted annotation proxy accepts text-only evidence".into()); }
        let bytes=serde_json::to_vec(payload).map_err(|e|e.to_string())?.len() as u64;
        if bytes>c.max_request_bytes { return Err("annotation request exceeds budgeted body limit".into()); }
        let output=payload["max_tokens"].as_u64().unwrap_or(c.max_output_tokens).min(c.max_output_tokens);
        payload["max_tokens"]=serde_json::json!(output);
        // UTF-8 bytes overbound text tokenization; double that plus explicit
        // protocol overhead to conservatively include message/tool framing.
        let input=bytes.saturating_mul(2).saturating_add(8192);
        let reserve=(input as f64*c.input_usd_per_million+output as f64*c.output_usd_per_million).ceil() as u64;
        let cap=(c.max_cost_usd*1_000_000.).floor() as u64;
        let parent=c.ledger_path.parent().ok_or("budget ledger has no directory")?;
        fs::create_dir_all(parent).map_err(|e|e.to_string())?;
        let lock=OpenOptions::new().create(true).truncate(false).write(true).open(c.ledger_path.with_extension("lock")).map_err(|e|e.to_string())?;
        lock.lock().map_err(|e|e.to_string())?;
        let mut ledger=match fs::read(&c.ledger_path) {
            Ok(raw)=>serde_json::from_slice::<Ledger>(&raw).map_err(|e|e.to_string())?,
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ledger{config:c.clone(),reserved_micros:0,requests:0,reserved_tokens:0},
            Err(e)=>return Err(e.to_string()),
        };
        if ledger.config!=*c { return Err("budget configuration changed for an existing ledger".into()); }
        let next=ledger.reserved_micros.checked_add(reserve).ok_or("budget overflow")?;
        if next>cap {return Err("annotation spending limit reached before provider request".into());}
        let tokens = ledger.reserved_tokens.checked_add(input).and_then(|n|n.checked_add(output)).ok_or("token budget overflow")?;
        if c.max_total_tokens.is_some_and(|limit|tokens > limit) { return Err("annotation token limit reached before provider request".into()); }
        ledger.reserved_tokens=tokens;ledger.reserved_micros=next;ledger.requests+=1;
        let staged=c.ledger_path.with_extension("pending");
        fs::write(&staged,serde_json::to_vec(&ledger).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        fs::File::open(&staged).and_then(|f|f.sync_all()).map_err(|e|e.to_string())?;
        fs::rename(staged,&c.ledger_path).map_err(|e|e.to_string())?;
        fs::File::open(parent).and_then(|f|f.sync_all()).map_err(|e|e.to_string())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_and_restarts_share_the_reserved_cap() {
        let root=std::env::temp_dir().join(format!("jesterky-budget-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let config=BudgetConfig{max_cost_usd:0.05,input_usd_per_million:1.,output_usd_per_million:10.,max_output_tokens:1024,max_request_bytes:16384,max_total_tokens:None,ledger_path:root.join("budget.json")};
        let mut payload=serde_json::json!({"model":"test","messages":[{"role":"user","content":"test"}],"max_tokens":100000});
        Budget::new(config.clone()).unwrap().reserve(&mut payload).unwrap();
        assert_eq!(payload["max_tokens"],1024);
        Budget::new(config.clone()).unwrap().reserve(&mut payload).unwrap();
        assert!(Budget::new(config).unwrap().reserve(&mut payload).is_err());
        let _=fs::remove_dir_all(root);
    }
    #[test]
    fn token_limit_persists_and_tool_schema_media_fields_are_text() {
        let root=std::env::temp_dir().join(format!("jesterky-token-budget-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let config=BudgetConfig{max_cost_usd:10.,input_usd_per_million:1.,output_usd_per_million:10.,max_output_tokens:1024,max_request_bytes:16384,max_total_tokens:Some(12000),ledger_path:root.join("budget.json")};
        let mut payload=serde_json::json!({"messages":[{"role":"user","content":"inspect text"}],"tools":[{"type":"function","function":{"name":"inspect","parameters":{"properties":{"audio":{"type":"string"}}}}}]});
        Budget::new(config.clone()).unwrap().reserve(&mut payload).unwrap();
        let ledger:Ledger=serde_json::from_slice(&fs::read(&config.ledger_path).unwrap()).unwrap();
        assert_eq!(ledger.requests,1);
        assert!(Budget::new(config.clone()).unwrap().reserve(&mut payload).unwrap_err().contains("token limit"));
        let unchanged:Ledger=serde_json::from_slice(&fs::read(&config.ledger_path).unwrap()).unwrap();
        assert_eq!(unchanged.requests,1);
        payload["messages"]=serde_json::json!([{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png"}}]}]);
        assert!(Budget::new(config).unwrap().reserve(&mut payload).unwrap_err().contains("text-only"));
        fs::remove_dir_all(root).unwrap();
    }

}
