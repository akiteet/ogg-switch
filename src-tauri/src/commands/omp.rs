use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{get_app_config_dir, get_home_dir};
use crate::services::session_usage_omp::OmpQuotaWindow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    /// OGG 专属 UI 数据，来源是 OGG 自己的 meta store；**不写入** models.yml
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub models: Vec<OmpModelInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<bool>,
    /// 解析自 models.yml 的原始 mapping；写回时 overlay 已知键、原样保留
    /// 未知键（omp 原生的 name / 自定义字段不丢失）。OGG 新建条目为 None。
    #[serde(skip)]
    pub raw: Option<YamlValue>,
    /// 来自 meta store 的排序值，仅供前端列表排序；不写 models.yml。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i64>,
    /// 库模式成员标记：true = 已在 models.yml / OAuth 合成；false = 仅存于库
    ///（meta.config 快照，显示「添加」按钮）。仅存在于 GUI 传输层，不写 models.yml。
    #[serde(default)]
    pub in_config: bool,
    /// 用量查询脚本配置。真源在 OGG meta store（与 sort_index 同理），
    /// 经 load_live_config 合入传输层；不写 models.yml。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<crate::provider::UsageScript>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub context_window: i64,
    #[serde(default)]
    pub max_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelRole {
    pub role: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmpSwitchConfig {
    pub version: i32,
    pub providers: Vec<OmpProviderConfig>,
    pub roles: Vec<OmpModelRole>,
}

fn omp_agent_dir() -> PathBuf {
    get_home_dir().join(".omp").join("agent")
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.exists())
}

fn models_path(dir: &Path) -> PathBuf {
    first_existing(dir, &["models.yml", "models.yaml"]).unwrap_or_else(|| dir.join("models.yml"))
}

fn config_path(dir: &Path) -> PathBuf {
    first_existing(dir, &["config.yml", "config.yaml"]).unwrap_or_else(|| dir.join("config.yml"))
}

fn infer_type(base_url: &str, api: &str) -> (String, String) {
    let url = base_url.to_lowercase();
    if url.contains("localhost") || url.contains("127.0.0.1") {
        return ("local".into(), "local".into());
    }
    if url.contains("openrouter")
        || url.contains("siliconflow")
        || url.contains("litellm")
        || url.contains(":3000")
        || url.contains("gateway")
    {
        return ("gateway".into(), "gateway".into());
    }
    if api.contains("anthropic") || api.contains("google") {
        return ("api-key".into(), "api".into());
    }
    ("api-key".into(), "api".into())
}

fn as_i64(value: &YamlValue) -> i64 {
    match value {
        YamlValue::Number(n) => n.as_i64().unwrap_or(0),
        YamlValue::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn parse_models_str(text: &str) -> Result<Vec<OmpProviderConfig>, String> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let root: YamlValue =
        serde_yaml::from_str(text).map_err(|e| format!("解析 models.yml 失败: {e}"))?;
    let Some(providers) = root.get("providers").and_then(|v| v.as_mapping()) else {
        return Ok(vec![]);
    };

    let mut out = Vec::new();
    for (id_key, raw) in providers {
        let Some(id) = id_key.as_str() else { continue };
        let table = raw.as_mapping().cloned().unwrap_or_default();
        let get = |key: &str| table.get(YamlValue::String(key.to_string()));
        let base_url = get("baseUrl")
            .or_else(|| get("base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let api = get("api")
            .and_then(|v| v.as_str())
            .unwrap_or("openai-completions")
            .to_string();
        let api_key = get("apiKey")
            .or_else(|| get("api_key"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let name = get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string();
        let (kind, category) = infer_type(&base_url, &api);
        let models = get("models")
            .and_then(|v| v.as_sequence())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let obj = item.as_mapping()?;
                        let mid = obj
                            .get(YamlValue::String("id".into()))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        let f = |key: &str| obj.get(YamlValue::String(key.into()));
                        Some(OmpModelInfo {
                            name: f("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&mid)
                                .to_string(),
                            api: f("api").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            reasoning: f("reasoning").and_then(|v| v.as_bool()),
                            context_window: f("contextWindow")
                                .or_else(|| f("context_window"))
                                .map(as_i64)
                                .unwrap_or(0),
                            max_tokens: f("maxTokens")
                                .or_else(|| f("max_tokens"))
                                .map(as_i64)
                                .unwrap_or(0),
                            id: mid,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let headers = get("headers").and_then(|v| serde_json::to_value(v).ok());
        let auth_header = get("authHeader")
            .or_else(|| get("auth_header"))
            .and_then(|v| v.as_bool());
        out.push(OmpProviderConfig {
            id: id.to_string(),
            name,
            r#type: kind,
            category,
            description: None,
            website_url: None,
            icon: None,
            models,
            oauth_provider_id: None,
            api: Some(api),
            base_url: if base_url.is_empty() {
                None
            } else {
                Some(base_url)
            },
            api_key,
            headers,
            auth_header,
            raw: Some(YamlValue::Mapping(table)),
            sort_index: None,
            in_config: true,
            usage_script: None,
        });
    }
    Ok(out)
}

fn parse_models_file(path: &Path) -> Result<Vec<OmpProviderConfig>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取 models.yml 失败: {e}"))?;
    parse_models_str(&text)
}

fn parse_role_selector(raw: &str) -> Option<(String, String, Option<String>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (model_part, thinking) = match trimmed.rsplit_once(':') {
        Some((left, right))
            if matches!(
                right,
                "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto"
            ) =>
        {
            (left, Some(right.to_string()))
        }
        _ => (trimmed, None),
    };
    let (provider, model) = model_part.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider.to_string(), model.to_string(), thinking))
}

fn parse_roles_str(text: &str) -> Result<Vec<OmpModelRole>, String> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let root: YamlValue =
        serde_yaml::from_str(text).map_err(|e| format!("解析 config.yml 失败: {e}"))?;
    let Some(roles) = root.get("modelRoles").and_then(|v| v.as_mapping()) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for (role_key, value) in roles {
        let (Some(role), Some(selector)) = (role_key.as_str(), value.as_str()) else {
            continue;
        };
        if let Some((provider_id, model_id, thinking_level)) = parse_role_selector(selector) {
            out.push(OmpModelRole {
                role: role.to_string(),
                provider_id,
                model_id,
                thinking_level,
            });
        }
    }
    Ok(out)
}

fn parse_roles_file(path: &Path) -> Result<Vec<OmpModelRole>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取 config.yml 失败: {e}"))?;
    parse_roles_str(&text)
}

fn merge_yaml_map(path: &Path, patch: BTreeMap<String, YamlValue>) -> Result<YamlValue, String> {
    let mut root = if path.exists() {
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.trim().is_empty() {
            YamlValue::Mapping(serde_yaml::Mapping::new())
        } else {
            serde_yaml::from_str(&text).unwrap_or(YamlValue::Mapping(serde_yaml::Mapping::new()))
        }
    } else {
        YamlValue::Mapping(serde_yaml::Mapping::new())
    };
    let obj = root
        .as_mapping_mut()
        .ok_or_else(|| "YAML 根节点必须是对象".to_string())?;
    for (key, value) in patch {
        obj.insert(YamlValue::String(key), value);
    }
    Ok(root)
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("替换配置文件失败: {e}"))?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// OGG 供应商元数据（icon / 显示名 / 备注 / 官网）
//
// 这些是 OGG 的 UI 数据，OMP 的 models.yml schema 不认识它们；写进真源会污染
// 用户配置。因此单独存 ~/.ogg-switch/omp_provider_meta.json（以 provider id
// 为键），读取 models.yml 时叠加覆盖，写回 models.yml 时只写 OMP 原生字段。
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpProviderMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 供应商类型（"oauth" / "api-key" ...）。OAuth 供应商不落 models.yml，
    /// 仅凭此字段在列表中合成条目。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// OAuth 供应商在 omp CLI 凭据库中的 id（如 anthropic / openai-codex）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// 拖拽排序持久化（OGG meta store 专属，不写入 models.yml）。
    /// None = 未排序（排在已排序条目之后，组内保持 yml 原序）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i64>,
    /// 库模式快照：保存供应商时的完整配置（含 apiKey）。移除仅将其撤出
    /// models.yml，库条目（meta）保留，可随时「添加」回来；彻底删除时清掉。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<OmpProviderConfig>,
    /// 用量查询脚本配置（OGG meta store 专属，不写 models.yml）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<crate::provider::UsageScript>,
}

pub type OmpProviderMetaMap = BTreeMap<String, OmpProviderMeta>;

fn omp_meta_path() -> PathBuf {
    get_app_config_dir().join("omp_provider_meta.json")
}

fn read_provider_meta() -> OmpProviderMetaMap {
    fs::read_to_string(omp_meta_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_provider_meta(map: &OmpProviderMetaMap) -> Result<(), String> {
    let text = serde_json::to_string_pretty(map).map_err(|e| format!("序列化 meta 失败: {e}"))?;
    atomic_write(&omp_meta_path(), &text)
}

/// 纯函数：从供应商配置提取需要持久化的 meta 条目。
/// 拖拽排序值不属于表单数据，从既有 meta 条目原样继承；
/// config 为库模式快照（完整配置），「移除」后仍可凭它「添加」回来。
fn meta_entry_from_provider(
    provider: &OmpProviderConfig,
    existing_sort_index: Option<i64>,
    existing_usage_script: Option<crate::provider::UsageScript>,
) -> OmpProviderMeta {
    let mut snapshot = provider.clone();
    snapshot.in_config = false;
    OmpProviderMeta {
        name: Some(provider.name.clone()),
        description: provider.description.clone(),
        website_url: provider.website_url.clone(),
        icon: provider.icon.clone(),
        provider_type: Some(provider.r#type.clone()),
        oauth_provider_id: provider.oauth_provider_id.clone(),
        api: provider.api.clone(),
        sort_index: existing_sort_index,
        usage_script: existing_usage_script.or(provider.usage_script.clone()),
        config: Some(snapshot),
    }
}

/// 纯函数：把 meta 覆盖进解析出的供应商列表（存在即覆盖；name 为空串不覆盖）。
fn apply_provider_meta(providers: &mut [OmpProviderConfig], meta: &OmpProviderMetaMap) {
    for provider in providers.iter_mut() {
        let Some(m) = meta.get(&provider.id) else {
            continue;
        };
        if let Some(name) = &m.name {
            if !name.trim().is_empty() {
                provider.name = name.clone();
            }
        }
        if m.description.is_some() {
            provider.description = m.description.clone();
        }
        if m.website_url.is_some() {
            provider.website_url = m.website_url.clone();
        }
        if m.icon.is_some() {
            provider.icon = m.icon.clone();
        }
        if m.sort_index.is_some() {
            provider.sort_index = m.sort_index;
        }
        // usage_script 真源在 meta store；Some 才覆盖（传输层回落 None）
        if m.usage_script.is_some() {
            provider.usage_script = m.usage_script.clone();
        }
    }
}

/// 纯函数：保存供应商时保持其在列表中的原位置（编辑不再把条目挪到末尾）。
fn insert_preserving_order(providers: &mut Vec<OmpProviderConfig>, provider: OmpProviderConfig) {
    let original_index = providers.iter().position(|p| p.id == provider.id);
    providers.retain(|p| p.id != provider.id);
    match original_index {
        Some(i) => providers.insert(i.min(providers.len()), provider),
        None => providers.push(provider),
    }
}

/// 已知键 overlay：`Some` 写入、`None` 移除（UI 清空字段即从 yml 移除该键）。
fn yaml_mapping_set(item: &mut serde_yaml::Mapping, key: &str, value: Option<YamlValue>) {
    let key = YamlValue::String(key.into());
    match value {
        Some(v) => {
            item.insert(key, v);
        }
        None => {
            item.remove(&key);
        }
    }
}

fn models_to_yaml_value(models: &[OmpModelInfo]) -> YamlValue {
    YamlValue::Sequence(
        models
            .iter()
            .map(|model| {
                let mut m = serde_yaml::Mapping::new();
                m.insert(
                    YamlValue::String("id".into()),
                    YamlValue::String(model.id.clone()),
                );
                // omp 的 schema 要求 name 非空；UI 允许留空（显示名可选），
                // 空缺时回落 id，否则写盘后 omp 会报 name must be non-empty。
                let display_name = if model.name.trim().is_empty() {
                    model.id.clone()
                } else {
                    model.name.clone()
                };
                m.insert(
                    YamlValue::String("name".into()),
                    YamlValue::String(display_name),
                );
                if let Some(api) = &model.api {
                    m.insert(
                        YamlValue::String("api".into()),
                        YamlValue::String(api.clone()),
                    );
                }
                if let Some(reasoning) = model.reasoning {
                    m.insert(
                        YamlValue::String("reasoning".into()),
                        YamlValue::Bool(reasoning),
                    );
                }
                if model.context_window > 0 {
                    m.insert(
                        YamlValue::String("contextWindow".into()),
                        YamlValue::Number(model.context_window.into()),
                    );
                }
                if model.max_tokens > 0 {
                    m.insert(
                        YamlValue::String("maxTokens".into()),
                        YamlValue::Number(model.max_tokens.into()),
                    );
                }
                YamlValue::Mapping(m)
            })
            .collect(),
    )
}

fn providers_to_yaml_value(providers: &[OmpProviderConfig]) -> YamlValue {
    let mut map = serde_yaml::Mapping::new();
    for provider in providers {
        // OAuth 供应商凭据由 omp CLI 凭据库管理，绝不写入 models.yml
        //（写入会因「有 models 无 baseUrl」触发 omp 校验失败）。
        if provider.r#type == "oauth" {
            continue;
        }
        // 库条目（已移除、仅存于 meta 快照）不是 live 配置，不得写回 models.yml，
        // 否则任何保存操作都会把「已移除」的供应商全部复活。
        if !provider.in_config {
            continue;
        }
        // raw 保真：保留 yml 原生未知键（name / 自定义字段），仅 overlay 已知键
        let mut item: serde_yaml::Mapping = provider
            .raw
            .as_ref()
            .and_then(|v| v.as_mapping().cloned())
            .unwrap_or_default();
        // provider 级 name：key 是内部 id（历史原因可能为 UUID），name 承载
        // 可读显示名（omp 原生可选字段）。仅在 raw 本就没有 name 键时写入
        //（原生条目的 name 已在 raw 中保真，不得被 meta 显示名覆盖）。
        if !item.contains_key(YamlValue::String("name".into())) {
            let display_name = provider.name.trim();
            yaml_mapping_set(
                &mut item,
                "name",
                (!display_name.is_empty()).then(|| YamlValue::String(display_name.to_string())),
            );
        }
        yaml_mapping_set(
            &mut item,
            "baseUrl",
            provider
                .base_url
                .clone()
                .filter(|s| !s.is_empty())
                .map(YamlValue::String),
        );
        yaml_mapping_set(
            &mut item,
            "apiKey",
            provider
                .api_key
                .clone()
                .filter(|s| !s.is_empty())
                .map(YamlValue::String),
        );
        yaml_mapping_set(
            &mut item,
            "api",
            provider
                .api
                .clone()
                .filter(|s| !s.is_empty())
                .map(YamlValue::String),
        );
        yaml_mapping_set(
            &mut item,
            "authHeader",
            provider.auth_header.map(YamlValue::Bool),
        );
        let headers_yaml = match &provider.headers {
            Some(h) if h.as_object().map(|o| !o.is_empty()).unwrap_or(false) => {
                serde_yaml::to_value(h).ok()
            }
            _ => None,
        };
        yaml_mapping_set(&mut item, "headers", headers_yaml);
        yaml_mapping_set(
            &mut item,
            "models",
            if provider.models.is_empty() {
                None
            } else {
                Some(models_to_yaml_value(&provider.models))
            },
        );
        map.insert(
            YamlValue::String(provider.id.clone()),
            YamlValue::Mapping(item),
        );
    }
    let mut root = serde_yaml::Mapping::new();
    root.insert(
        YamlValue::String("providers".into()),
        YamlValue::Mapping(map),
    );
    YamlValue::Mapping(root)
}

fn write_live_config(config: &OmpSwitchConfig) -> Result<(), String> {
    let dir = omp_agent_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建 ~/.omp/agent: {e}"))?;
    let models = models_path(&dir);
    let cfg = config_path(&dir);

    let models_yaml = serde_yaml::to_string(&providers_to_yaml_value(&config.providers))
        .map_err(|e| format!("序列化 models.yml 失败: {e}"))?;
    atomic_write(&models, &models_yaml)?;

    let mut roles = serde_yaml::Mapping::new();
    for role in &config.roles {
        let selector = match &role.thinking_level {
            Some(level) if !level.is_empty() => {
                format!("{}/{}:{}", role.provider_id, role.model_id, level)
            }
            _ => format!("{}/{}", role.provider_id, role.model_id),
        };
        roles.insert(
            YamlValue::String(role.role.clone()),
            YamlValue::String(selector),
        );
    }
    let mut patch = BTreeMap::new();
    patch.insert("modelRoles".into(), YamlValue::Mapping(roles));
    let merged = merge_yaml_map(&cfg, patch)?;
    let config_yaml =
        serde_yaml::to_string(&merged).map_err(|e| format!("序列化 config.yml 失败: {e}"))?;
    atomic_write(&cfg, &config_yaml)?;
    Ok(())
}

/// 旧版 OGG 预设 id → omp 内置目录 id（`~/.omp/agent/models.db` 的 provider_id 实证）。
/// 这些预设与内置供应商同服务同端点，仅键名不同；models.yml 落旧键会与内置
/// 并存成两个供应商（如 deepseek-api + deepseek），加载前统一改名。
const LEGACY_PROVIDER_ID_RENAMES: &[(&str, &str)] = &[
    ("anthropic-api", "anthropic"),
    ("openai-api", "openai"),
    ("google-api", "google"),
    ("xai-api", "xai"),
    ("deepseek-api", "deepseek"),
    ("mistral-api", "mistral"),
    ("together-api", "together"),
];

/// 启动迁移：models.yml / config.yml / meta 中旧版预设 id 改名为内置 id。
/// 幂等：仅当目标键不存在时改名；任一文件变化才写回；失败不阻断加载。
fn migrate_legacy_provider_ids() {
    if let Err(err) = try_migrate_legacy_provider_ids() {
        log::warn!("迁移旧版 omp 供应商 id 失败（忽略，不影响加载）: {err}");
    }
}

/// 静态旧版预设映射的动态视图（供泛化后的 rename 函数使用）
fn legacy_mapping() -> Vec<(String, String)> {
    LEGACY_PROVIDER_ID_RENAMES
        .iter()
        .map(|(old, new)| ((*old).to_string(), (*new).to_string()))
        .collect()
}

fn try_migrate_legacy_provider_ids() -> Result<(), String> {
    let dir = omp_agent_dir();

    // 1. models.yml：providers.<old> → <new>
    let models = models_path(&dir);
    if models.exists() {
        let text = fs::read_to_string(&models).map_err(|e| format!("读取 models.yml 失败: {e}"))?;
        let root: YamlValue = serde_yaml::from_str(&text).unwrap_or(YamlValue::Null);
        let (updated, changed) =
            rename_models_yaml_provider_keys_with(&root, &legacy_mapping(), true);
        if changed {
            let out = serde_yaml::to_string(&updated)
                .map_err(|e| format!("序列化 models.yml 失败: {e}"))?;
            atomic_write(&models, &out)?;
        }
    }

    // 2. config.yml：modelRoles / modelProviderOrder / retry.fallbackChains
    let config = config_path(&dir);
    if config.exists() {
        let text = fs::read_to_string(&config).map_err(|e| format!("读取 config.yml 失败: {e}"))?;
        let root: YamlValue = serde_yaml::from_str(&text).unwrap_or(YamlValue::Null);
        let (updated, changed) = rename_config_yaml_provider_refs_with(&root, &legacy_mapping());
        if changed {
            let out = serde_yaml::to_string(&updated)
                .map_err(|e| format!("序列化 config.yml 失败: {e}"))?;
            atomic_write(&config, &out)?;
        }
    }

    // 3. meta store：键改名（内容保留）
    if omp_meta_path().exists() {
        let meta = read_provider_meta();
        let (renamed, changed) = rename_meta_keys_with(&meta, &legacy_mapping());
        if changed {
            write_provider_meta(&renamed)?;
        }
    }
    Ok(())
}

/// 启动迁移：历史 bug 修复——纯中文名供应商 slug 化为空会回落 UUID 作为
/// models.yml key，导致 omp /model 与角色 selector 显示 UUID。这里按
/// meta store 中的显示名把 UUID 键改成可读键（幂等：目标键已占用则跳过）。
fn migrate_uuid_provider_keys() {
    if let Err(err) = try_migrate_uuid_provider_keys() {
        log::warn!("迁移 UUID 供应商键失败（忽略，不影响加载）: {err}");
    }
}

fn try_migrate_uuid_provider_keys() -> Result<(), String> {
    let dir = omp_agent_dir();
    let models = models_path(&dir);
    if !models.exists() || !omp_meta_path().exists() {
        return Ok(());
    }
    // 现有键集合（models.yml 的 provider key + OAuth 合成条目 id）
    let existing_keys: std::collections::HashSet<String> = parse_models_file(&models)?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let meta = read_provider_meta();

    // 构建 UUID → 显示名 映射；目标名被占用（重名）时跳过该条，保留 UUID
    let mut claimed: std::collections::HashSet<String> = existing_keys.clone();
    let mut mapping: Vec<(String, String)> = Vec::new();
    for (id, entry) in &meta {
        if !looks_like_uuid(id) {
            continue;
        }
        let Some(name) = entry
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.contains('/'))
        else {
            continue;
        };
        if claimed.contains(name) {
            continue;
        }
        claimed.insert(name.to_string());
        mapping.push((id.clone(), name.to_string()));
    }
    if mapping.is_empty() {
        return Ok(());
    }

    // 1. models.yml：UUID 键 → 显示名（目标已存在则跳过，绝不丢条目）
    let text = fs::read_to_string(&models).map_err(|e| format!("读取 models.yml 失败: {e}"))?;
    let root: YamlValue = serde_yaml::from_str(&text).unwrap_or(YamlValue::Null);
    let (updated, changed) = rename_models_yaml_provider_keys_with(&root, &mapping, false);
    if changed {
        let out =
            serde_yaml::to_string(&updated).map_err(|e| format!("序列化 models.yml 失败: {e}"))?;
        atomic_write(&models, &out)?;
    }

    // 2. config.yml 引用改名
    let config = config_path(&dir);
    if config.exists() {
        let text = fs::read_to_string(&config).map_err(|e| format!("读取 config.yml 失败: {e}"))?;
        let root: YamlValue = serde_yaml::from_str(&text).unwrap_or(YamlValue::Null);
        let (updated, changed) = rename_config_yaml_provider_refs_with(&root, &mapping);
        if changed {
            let out = serde_yaml::to_string(&updated)
                .map_err(|e| format!("序列化 config.yml 失败: {e}"))?;
            atomic_write(&config, &out)?;
        }
    }

    // 3. meta store 键改名
    let (renamed, changed) = rename_meta_keys_with(&meta, &mapping);
    if changed {
        write_provider_meta(&renamed)?;
    }
    Ok(())
}

/// 判断 key 是否为 UUID 形态（8-4-4-4-12 十六进制）
fn looks_like_uuid(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, b)| match i {
        8 | 13 | 18 | 23 => *b == b'-',
        _ => b.is_ascii_hexdigit(),
    })
}

/// 纯函数：`old` 或 `old/…` 形态的字符串按映射改前缀；未命中返回 None。
fn rename_provider_prefix_with(value: &str, mapping: &[(String, String)]) -> Option<String> {
    for (old, new) in mapping {
        if value == old {
            return Some(new.clone());
        }
        let prefix = format!("{old}/");
        if let Some(rest) = value.strip_prefix(&prefix) {
            return Some(format!("{new}/{rest}"));
        }
    }
    None
}

/// 纯函数：models.yml 顶层 providers mapping 的键改名。
/// 目标键已存在时丢弃旧条目（以内置为准），仍视为变更。
fn rename_models_yaml_provider_keys_with(
    root: &YamlValue,
    mapping: &[(String, String)],
    drop_on_conflict: bool,
) -> (YamlValue, bool) {
    let mut out = root.clone();
    let mut changed = false;
    let Some(providers) = out
        .as_mapping_mut()
        .and_then(|map| map.get_mut(YamlValue::String("providers".into())))
        .and_then(|v| v.as_mapping_mut())
    else {
        return (out, false);
    };
    for (old, new) in mapping {
        let old_key = YamlValue::String(old.clone());
        if !providers.contains_key(&old_key) {
            continue;
        }
        let new_key = YamlValue::String(new.clone());
        let conflict = providers.contains_key(&new_key);
        if conflict && !drop_on_conflict {
            // UUID 迁移语义：目标键被占 → 跳过，保留旧条目
            continue;
        }
        let value = providers.remove(&old_key).expect("checked above");
        changed = true;
        if !conflict {
            providers.insert(new_key, value);
        }
    }
    (out, changed)
}

/// 纯函数：config.yml 中指向旧 id 的引用改名。
/// - `modelRoles`：值选择器 `old/model[:level]` → `new/…`
/// - `modelProviderOrder`：序列项 `old` → `new`
/// - `retry.fallbackChains`：键 `old` / `old/*` 与值序列中的 `old/…` → `new/…`
fn rename_config_yaml_provider_refs_with(
    root: &YamlValue,
    mapping: &[(String, String)],
) -> (YamlValue, bool) {
    let mut out = root.clone();
    let mut changed = false;

    if let Some(map) = out.as_mapping_mut() {
        if let Some(roles) = map
            .get_mut(YamlValue::String("modelRoles".into()))
            .and_then(|v| v.as_mapping_mut())
        {
            for (_role, selector) in roles.iter_mut() {
                if let Some(text) = selector.as_str() {
                    if let Some(renamed) = rename_provider_prefix_with(text, mapping) {
                        *selector = YamlValue::String(renamed);
                        changed = true;
                    }
                }
            }
        }

        if let Some(order) = map
            .get_mut(YamlValue::String("modelProviderOrder".into()))
            .and_then(|v| v.as_sequence_mut())
        {
            for item in order.iter_mut() {
                if let Some(text) = item.as_str() {
                    if let Some(renamed) = rename_provider_prefix_with(text, mapping) {
                        *item = YamlValue::String(renamed);
                        changed = true;
                    }
                }
            }
        }

        if let Some(chains_map) = map
            .get_mut(YamlValue::String("retry".into()))
            .and_then(|v| v.as_mapping_mut())
            .and_then(|retry| retry.get_mut(YamlValue::String("fallbackChains".into())))
            .and_then(|v| v.as_mapping_mut())
        {
            // 键改名：先收集再替换，避免边遍历边改键
            let stale: Vec<(YamlValue, YamlValue, YamlValue)> = chains_map
                .iter()
                .filter_map(|(k, v)| {
                    let new_key = k
                        .as_str()
                        .and_then(|text| rename_provider_prefix_with(text, mapping))?;
                    Some((k.clone(), YamlValue::String(new_key), v.clone()))
                })
                .collect();
            for (old_key, new_key, value) in stale {
                chains_map.remove(&old_key);
                changed = true;
                if !chains_map.contains_key(&new_key) {
                    chains_map.insert(new_key, value);
                }
            }
            // 值序列中的选择器改名
            for (_key, value) in chains_map.iter_mut() {
                if let Some(seq) = value.as_sequence_mut() {
                    for item in seq.iter_mut() {
                        if let Some(text) = item.as_str() {
                            if let Some(renamed) = rename_provider_prefix_with(text, mapping) {
                                *item = YamlValue::String(renamed);
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    (out, changed)
}

/// 纯函数：meta store 键改名（目标键已存在时保留已有条目，丢弃旧条目）。
fn rename_meta_keys_with(
    meta: &OmpProviderMetaMap,
    mapping: &[(String, String)],
) -> (OmpProviderMetaMap, bool) {
    let mut out = meta.clone();
    let mut changed = false;
    for (old, new) in mapping {
        if let Some(entry) = out.remove(old) {
            changed = true;
            out.entry(new.clone()).or_insert(entry);
        }
    }
    (out, changed)
}

fn load_live_config() -> Result<OmpSwitchConfig, String> {
    migrate_legacy_provider_ids();
    migrate_uuid_provider_keys();
    let dir = omp_agent_dir();
    let mut providers = parse_models_file(&models_path(&dir))?;
    let roles = parse_roles_file(&config_path(&dir))?;
    let meta = read_provider_meta();
    apply_provider_meta(&mut providers, &meta);
    synthesize_oauth_providers(&mut providers, &meta);
    // 库条目：已从配置移除但保留在库（meta.config 快照）的供应商，供「添加」
    synthesize_library_providers(&mut providers, &meta);
    // 拖拽排序：按 meta 的 sort_index 稳定排序；未排序条目排在后面，
    // 组内保持 models.yml 原序（新增供应商自然落在末尾）。
    providers.sort_by_key(|p| p.sort_index.unwrap_or(i64::MAX));
    Ok(OmpSwitchConfig {
        version: 1,
        providers,
        roles,
    })
}

/// 纯函数：meta 中记录为 OAuth、且 yml 无对应条目的供应商，合成列表项展示。
/// 凭据在 omp CLI 凭据库中，models.yml 不落盘；模型列表留空（由 omp 从上游
/// 自动发现，角色选择器直接引用上游模型 id 即可）。
fn synthesize_oauth_providers(providers: &mut Vec<OmpProviderConfig>, meta: &OmpProviderMetaMap) {
    for (id, m) in meta {
        if m.provider_type.as_deref() != Some("oauth") {
            continue;
        }
        if providers.iter().any(|p| p.id == *id) {
            continue;
        }
        providers.push(OmpProviderConfig {
            id: id.clone(),
            name: m
                .name
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| id.clone()),
            r#type: "oauth".into(),
            // 必须落在 OmpProviderCategory 的合法值内（queries.ts 会据此映射
            // 成 official 分类），不能自造 "oauth" 值。
            category: "subscription".into(),
            description: m.description.clone(),
            website_url: m.website_url.clone(),
            icon: m.icon.clone(),
            models: Vec::new(),
            oauth_provider_id: m.oauth_provider_id.clone().or_else(|| Some(id.clone())),
            api: m.api.clone(),
            base_url: None,
            api_key: None,
            headers: None,
            auth_header: None,
            raw: None,
            sort_index: m.sort_index,
            in_config: true,
            usage_script: None,
        });
    }
}

/// 纯函数：库条目合成。meta 中存有完整配置快照（config）、非 OAuth、且 yml
/// 无对应条目的供应商 = 「已从配置移除但保留在库中」，合成进列表供「添加」。
/// live 条目与 OAuth 合成条目不受影响（in_config 已为 true）。
fn synthesize_library_providers(providers: &mut Vec<OmpProviderConfig>, meta: &OmpProviderMetaMap) {
    for (id, m) in meta {
        let Some(mut config) = m.config.clone() else {
            continue;
        };
        if config.r#type == "oauth" {
            // OAuth 条目不做库化：凭据在 omp 凭据库，移除即删除 meta
            continue;
        }
        if providers.iter().any(|p| p.id == *id) {
            continue;
        }
        // meta 的 UI 字段（名称/图标/排序）以最新值为准
        if let Some(name) = m.name.clone().filter(|s| !s.trim().is_empty()) {
            config.name = name;
        }
        if m.icon.is_some() {
            config.icon = m.icon.clone();
        }
        config.sort_index = m.sort_index;
        config.in_config = false;
        providers.push(config);
    }
}

/// 解析 omp CLI 可执行文件（misc.rs 的搜索路径体系覆盖 ~/.bun/bin 等安装位，
/// 避免 GUI 进程 PATH 残缺时裸 `omp` 静默失败）。
fn omp_exe() -> PathBuf {
    crate::commands::misc::locate_omp_command()
}

/// GUI 进程生成控制台子进程必须抑制窗口：不加此 flag，每次 `omp --version` /
/// `omp token --list` / secret-bridge 解密都会新建一个终端窗口——OAuth 表单 3 秒
/// 轮询时就是「终端窗口反复弹出」；且闪窗被关闭会给子进程投递终止信号
/// （日志实测 exit 0xC000013A STATUS_CONTROL_C_EXIT），探测永远失败、登录状态
/// 永远回填不上。与 misc.rs 既有的 CREATE_NO_WINDOW 模式对齐。
fn spawn_headless(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW)
    }
    #[cfg(not(target_os = "windows"))]
    {
        cmd
    }
}

fn omp_cli_available() -> bool {
    let exe = omp_exe();
    let mut cmd = Command::new(&exe);
    cmd.arg("--version");
    match spawn_headless(&mut cmd).output() {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            log::warn!(
                "omp CLI 探测失败（{}）：exit={:?} stderr={}",
                exe.display(),
                o.status.code(),
                String::from_utf8_lossy(&o.stderr)
                    .trim()
                    .chars()
                    .take(200)
                    .collect::<String>()
            );
            false
        }
        Err(e) => {
            log::warn!("omp CLI 启动失败（{}）：{e}", exe.display());
            false
        }
    }
}

#[tauri::command]
pub async fn read_omp_config() -> Result<OmpSwitchConfig, String> {
    load_live_config()
}

#[tauri::command]
pub async fn save_omp_provider(mut provider: OmpProviderConfig) -> Result<(), String> {
    // meta 字段（icon/名称/备注/官网）落 OGG 自己的 store；models.yml 只写 OMP 原生字段
    let mut meta = read_provider_meta();
    let existing = meta.get(&provider.id);
    let existing_sort_index = existing.and_then(|m| m.sort_index);
    let existing_usage_script = existing.and_then(|m| m.usage_script.clone());
    meta.insert(
        provider.id.clone(),
        meta_entry_from_provider(&provider, existing_sort_index, existing_usage_script),
    );
    write_provider_meta(&meta)?;

    // OAuth 供应商凭据由 omp CLI 凭据库管理，绝不写入 models.yml
    //（写入会因「有 models 无 baseUrl」触发 omp 校验失败，见真实报错）。
    if provider.r#type == "oauth" {
        // 清理历史污染：早期版本会把 OAuth 供应商写进 models.yml（带 models 却无
        // baseUrl），导致 omp 校验失败、整个供应商列表不可用。这里顺手移除同名条目。
        let dir = omp_agent_dir();
        let on_disk = parse_models_file(&models_path(&dir))?;
        if on_disk.iter().any(|p| p.id == provider.id) {
            let mut config = load_live_config()?;
            config.providers.retain(|p| p.id != provider.id);
            write_live_config(&config)?;
        }
        return Ok(());
    }

    let mut config = load_live_config()?;
    // 保存 = 添加/更新到 live 配置：无论载荷来自新建表单还是库条目（in_config=false），
    // 落 yml 前必须置回 true，否则会被 write_live_config 的库条目过滤跳过。
    provider.in_config = true;
    // 编辑态的前端载荷不含 raw（yml 原生 mapping 只有后端读得到）。不继承的话，
    // 该条目在 models.yml 里的未知键（omp 原生 name、用户自定义字段）会被抹掉。
    if let Some(existing) = config.providers.iter().find(|p| p.id == provider.id) {
        provider.raw = existing.raw.clone();
    }
    insert_preserving_order(&mut config.providers, provider);
    write_live_config(&config)
}

#[tauri::command]
pub async fn delete_omp_provider(provider_id: String) -> Result<(), String> {
    let mut config = load_live_config()?;
    config.providers.retain(|p| p.id != provider_id);
    config.roles.retain(|r| r.provider_id != provider_id);
    write_live_config(&config)?;
    // config.yml 里还有 omp 自己写入、指向该供应商的引用（retry.fallbackChains
    // 的通配键、modelProviderOrder 的列表项）。OGG 不会重建它们，残留会让 omp
    // 启动时报 "references unknown provider/model"，所以删除时一并收拾。
    if let Err(err) = cleanup_config_references(&provider_id) {
        // 清理失败不阻断删除：供应商已从 models.yml 移除，此处回滚更糟
        log::warn!("清理 config.yml 中对 {provider_id} 的引用失败: {err}");
    }
    // 同步清掉 OGG meta，避免删除后重建同名供应商时读到陈旧图标/名称
    let mut meta = read_provider_meta();
    if meta.remove(&provider_id).is_some() {
        write_provider_meta(&meta)?;
    }
    Ok(())
}

/// 「移除」= 仅撤出 live 配置（models.yml + roles + config 引用），
/// meta 库条目（含 config 快照）保留——前端列表改显示「添加」，可随时加回。
/// 与 delete_omp_provider（彻底删除，含 meta）构成移除/删除两级语义。
#[tauri::command]
pub async fn remove_omp_provider_from_live(provider_id: String) -> Result<(), String> {
    let mut config = load_live_config()?;
    // 移除前确保库快照存在：存量供应商（旧版本保存）的 meta 无 config 字段，
    // 直接移除会导致条目从列表消失（表现为删除而非移除）。
    if let Some(live_entry) = config
        .providers
        .iter()
        .find(|p| p.id == provider_id && p.r#type != "oauth")
        .cloned()
    {
        let meta = read_provider_meta();
        let (meta, changed) = ensure_library_snapshot(meta, &live_entry);
        if changed {
            write_provider_meta(&meta)?;
        }
    }
    config.providers.retain(|p| p.id != provider_id);
    config.roles.retain(|r| r.provider_id != provider_id);
    write_live_config(&config)?;
    // config.yml 的悬空引用清理与 delete 相同（残留会让 omp 启动报错）
    if let Err(err) = cleanup_config_references(&provider_id) {
        log::warn!("清理 config.yml 中对 {provider_id} 的引用失败: {err}");
    }
    Ok(())
}

/// 纯函数：确保 meta 中存在 provider 的库快照。返回 (新 map, 是否有变更)。
/// 存量供应商（第八轮前保存）的 meta 无 config 字段，直接移除会表现为删除；
/// 从 live 条目补写快照，保证「移除后仍显示在列表（添加状态）」。
fn ensure_library_snapshot(
    mut meta: OmpProviderMetaMap,
    provider: &OmpProviderConfig,
) -> (OmpProviderMetaMap, bool) {
    // OAuth 无成员语义（凭据在 omp 凭据库，每次 load 都会重新合成），不做库化
    if provider.r#type == "oauth" {
        return (meta, false);
    }
    if meta
        .get(&provider.id)
        .map(|m| m.config.is_some())
        .unwrap_or(false)
    {
        // 已有快照（第八轮起 save 写入），保留最新，不覆盖
        return (meta, false);
    }
    let existing_sort_index = meta.get(&provider.id).and_then(|m| m.sort_index);
    meta.insert(
        provider.id.clone(),
        meta_entry_from_provider(provider, existing_sort_index, None),
    );
    (meta, true)
}

/// 从 config.yml 移除指向指定供应商的悬空引用，返回被清理的条目数。
fn cleanup_config_references(provider_id: &str) -> Result<usize, String> {
    let dir = omp_agent_dir();
    let path = config_path(&dir);
    if !path.exists() {
        return Ok(0);
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 config.yml 失败: {e}"))?;
    let cfg: YamlValue = serde_yaml::from_str(&text).unwrap_or(YamlValue::Null);
    let (updated, removed) = strip_provider_references(&cfg, provider_id);
    if removed == 0 {
        return Ok(0);
    }
    let out =
        serde_yaml::to_string(&updated).map_err(|e| format!("序列化 config.yml 失败: {e}"))?;
    atomic_write(&path, &out)?;
    Ok(removed)
}

/// 纯函数：删除 config.yml 中指向 `provider_id` 的引用。
///
/// - `retry.fallbackChains`：移除键为 `{id}` 或 `{id}/…`（如 `SenseNova/*`）的条目
/// - `modelProviderOrder`：从字符串序列中移除该 id
///
/// 只动这两处已知会携带 provider 引用的键，其余配置（autolearn 等）原样保留。
fn strip_provider_references(cfg: &YamlValue, provider_id: &str) -> (YamlValue, usize) {
    let mut root = cfg.clone();
    let mut removed = 0usize;
    let Some(map) = root.as_mapping_mut() else {
        return (root, 0);
    };

    if let Some(retry) = map.get_mut(YamlValue::String("retry".into())) {
        if let Some(retry_map) = retry.as_mapping_mut() {
            if let Some(chains) = retry_map.get_mut(YamlValue::String("fallbackChains".into())) {
                if let Some(chains_map) = chains.as_mapping_mut() {
                    let prefix = format!("{provider_id}/");
                    let stale: Vec<YamlValue> = chains_map
                        .keys()
                        .filter(|k| {
                            k.as_str()
                                .is_some_and(|s| s == provider_id || s.starts_with(&prefix))
                        })
                        .cloned()
                        .collect();
                    for key in stale {
                        chains_map.remove(&key);
                        removed += 1;
                    }
                }
            }
        }
    }

    if let Some(order) = map.get_mut(YamlValue::String("modelProviderOrder".into())) {
        if let Some(seq) = order.as_sequence_mut() {
            let before = seq.len();
            seq.retain(|v| v.as_str() != Some(provider_id));
            removed += before - seq.len();
        }
    }

    (root, removed)
}

#[tauri::command]
pub async fn get_all_omp_providers() -> Result<Vec<OmpProviderConfig>, String> {
    Ok(load_live_config()?.providers)
}

/// 把 omp 供应商转换为通用 Provider 结构（用量查询等通用体系复用）。
/// settings_config 走 {"config": "<OmpProviderConfig JSON>"} 载体，
/// 与 provider.rs resolve_usage_credentials 的 Omp 分支口径一致。
pub(crate) fn omp_provider_to_usage_provider(
    config: OmpProviderConfig,
) -> crate::provider::Provider {
    let config_json = serde_json::to_string(&config).unwrap_or_default();
    let mut provider = crate::provider::Provider::with_id(
        config.id.clone(),
        config.name.clone(),
        serde_json::json!({ "config": config_json }),
        config.website_url.clone(),
    );
    provider.category = Some("custom".to_string());
    if config.usage_script.is_some() {
        provider.meta = Some(crate::provider::ProviderMeta {
            usage_script: config.usage_script.clone(),
            ..crate::provider::ProviderMeta::default()
        });
    }
    provider
}

/// 查找单个 omp 供应商并转换为通用 Provider（不在 SQLite，专道构造）。
pub(crate) async fn find_omp_usage_provider(
    provider_id: &str,
) -> Result<crate::provider::Provider, crate::error::AppError> {
    let providers = get_all_omp_providers()
        .await
        .map_err(crate::error::AppError::Message)?;
    let matched = providers
        .into_iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| {
            crate::error::AppError::localized(
                "provider.not_found",
                format!("供应商不存在: {provider_id}"),
                format!("Provider not found: {provider_id}"),
            )
        })?;
    Ok(omp_provider_to_usage_provider(matched))
}

/// 保存 omp 供应商的用量查询脚本（真源 = OGG meta store，仿 pi 专道；
/// omp 不在 SQLite，通用 update_provider 命令不适用）。
#[tauri::command]
pub async fn update_omp_provider_usage_script(
    id: String,
    usage_script: crate::provider::UsageScript,
) -> Result<bool, String> {
    let mut meta = read_provider_meta();
    let entry = meta.entry(id).or_default();
    entry.usage_script = Some(usage_script);
    write_provider_meta(&meta)?;
    Ok(true)
}

/// 拖拽排序持久化：按前端提交的完整可见顺序把 sort_index 写入 meta store。
/// models.yml 不动（omp 原生文件不携带 OGG 排序概念），列表顺序由
/// load_live_config 统一应用。
#[tauri::command]
pub async fn set_omp_providers_order(ids: Vec<String>) -> Result<(), String> {
    let mut meta = read_provider_meta();
    for (index, id) in ids.into_iter().enumerate() {
        let entry = meta.entry(id).or_default();
        entry.sort_index = Some(index as i64);
    }
    write_provider_meta(&meta)
}

#[tauri::command]
pub async fn set_omp_role(role_assignment: OmpModelRole) -> Result<(), String> {
    let mut config = load_live_config()?;
    config.roles.retain(|r| r.role != role_assignment.role);
    config.roles.push(role_assignment);
    write_live_config(&config)
}

#[tauri::command]
pub async fn delete_omp_role(role: String) -> Result<(), String> {
    let mut config = load_live_config()?;
    config.roles.retain(|r| r.role != role);
    write_live_config(&config)
}

// ────────────────────────────────────────────────────────────────────────────
// 托盘集成：OMP 不在 AppType/SQLite 供应商体系内，「当前供应商」由
// config.yml:modelRoles.default 决定，故托盘分区单独走这两个入口。
// ────────────────────────────────────────────────────────────────────────────

/// 托盘 Oh My Pi 分区数据。
pub struct OmpTraySnapshot {
    /// (provider id, 显示名)
    pub providers: Vec<(String, String)>,
    /// 当前默认供应商 id（modelRoles.default 的 provider 部分，可能为空）
    pub current_provider_id: String,
}

/// 读取托盘分区所需的 OMP 供应商列表与当前默认。OMP 未配置时 providers 为空。
pub fn omp_tray_snapshot() -> Result<OmpTraySnapshot, String> {
    let config = load_live_config()?;
    let current_provider_id = config
        .roles
        .iter()
        .find(|r| r.role == "default")
        .map(|r| r.provider_id.clone())
        .unwrap_or_default();
    Ok(OmpTraySnapshot {
        providers: config
            .providers
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect(),
        current_provider_id,
    })
}

/// 托盘「设为默认」：把 modelRoles.default 写成 `<providerId>/<首个模型>`。
/// 供应商没有模型记录时放弃——写不出合法 selector（OMP 无法据此路由）。
pub fn omp_set_default_provider(provider_id: &str) -> Result<(), String> {
    let mut config = load_live_config()?;
    let Some(provider) = config.providers.iter().find(|p| p.id == provider_id) else {
        return Err(format!("OMP 供应商不存在: {provider_id}"));
    };
    let Some(model) = provider.models.first() else {
        return Err(format!(
            "OMP 供应商 {provider_id} 没有可用模型，无法设为默认"
        ));
    };
    config.roles.retain(|r| r.role != "default");
    config.roles.push(OmpModelRole {
        role: "default".into(),
        provider_id: provider_id.to_string(),
        model_id: model.id.clone(),
        thinking_level: None,
    });
    write_live_config(&config)
}

// ────────────────────────────────────────────────────────────────────────────
// 认证：驱动本机 omp CLI 的凭据库（不自建第二套凭据，否则 OMP 读不到）
//
// 实测事实（omp v18.x）：
//   omp token <provider> --list   列该 provider 的 OAuth 账号
//   omp auth-broker login <p>     交互式 OAuth（无需 broker 时直接用本地库）
//   omp auth-broker logout <p>    移除凭据
//   omp auth-broker status --json 集中式 broker 健康状态（默认 not_configured）
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpAuthAccount {
    /// 1-based，与 `omp token <p> --account N` 对齐
    pub index: i64,
    pub identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpAuthStatus {
    pub cli_available: bool,
    pub logged_in: bool,
    pub accounts: Vec<OmpAuthAccount>,
    pub message: String,
}

fn run_omp(args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(omp_exe());
    cmd.args(args);
    spawn_headless(&mut cmd)
        .output()
        .map_err(|e| format!("执行 omp 失败: {e}"))
}

/// 解析 `omp token <provider> --list` 的文本输出。
/// 无账号时输出 `No OAuth accounts found for provider "x".`。
/// 有账号时每行形如 `#1  user@example.com`（不同版本可能略有差异，故按行宽松解析）。
fn parse_token_accounts(stdout: &str) -> Vec<OmpAuthAccount> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with("No OAuth accounts")
            || t.starts_with("--account")
            || t.starts_with("Use ")
        {
            continue;
        }
        // 提取可选的 "#N" 前缀
        let (index, rest) = if let Some(after) = t.strip_prefix('#') {
            let mut parts = after.splitn(2, char::is_whitespace);
            let num = parts.next().unwrap_or("").trim();
            match num.parse::<i64>() {
                Ok(n) => (n, parts.next().unwrap_or("").trim().to_string()),
                Err(_) => (0, t.to_string()),
            }
        } else {
            (0, t.to_string())
        };
        if rest.is_empty() {
            continue;
        }
        out.push(OmpAuthAccount {
            index: if index > 0 {
                index
            } else {
                (out.len() as i64) + 1
            },
            identity: rest,
        });
    }
    out
}

/// 查询某 provider 的真实登录状态与账号列表。
#[tauri::command]
pub async fn omp_auth_status(provider_id: String) -> Result<OmpAuthStatus, String> {
    if !omp_cli_available() {
        // 带上探测的具体可执行路径：前端横幅会展示这行，日志里有更详细的
        // 定位/退出码信息（locate_omp_command / omp_cli_available 的 warn）。
        let exe = omp_exe();
        return Ok(OmpAuthStatus {
            cli_available: false,
            logged_in: false,
            accounts: vec![],
            message: format!(
                "未检测到 Oh My Pi CLI（探测 {} 失败，详见应用日志）。",
                exe.display()
            ),
        });
    }
    let provider = provider_id.trim();
    if provider.is_empty() {
        return Ok(OmpAuthStatus {
            cli_available: true,
            logged_in: false,
            accounts: vec![],
            message: "未指定 OAuth Provider ID。".into(),
        });
    }
    let output = run_omp(&["token", provider, "--list"])?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let accounts = parse_token_accounts(&stdout);
    let logged_in = !accounts.is_empty();
    let message = if logged_in {
        format!("已登录（{} 个账号）", accounts.len())
    } else if stdout.contains("No OAuth accounts") || output.status.success() {
        format!("{provider} 尚未登录")
    } else {
        stderr.trim().to_string()
    };
    Ok(OmpAuthStatus {
        cli_available: true,
        logged_in,
        accounts,
        message,
    })
}

/// 在独立终端窗口启动 `omp auth-broker login <provider>`，由用户在 OMP 原生
/// 流程里完成 OAuth（浏览器 / 设备码 / 粘贴回调，取决于 provider）。
/// 完成后前端轮询 omp_auth_status 回填状态。
#[tauri::command]
pub async fn omp_auth_login(provider_id: String) -> Result<(), String> {
    if !omp_cli_available() {
        return Err("未检测到 Oh My Pi CLI，请先安装 omp。".into());
    }
    let provider = provider_id.trim();
    if provider.is_empty() {
        return Err("未指定 OAuth Provider ID。".into());
    }
    // 终端里同样要用解析后的完整路径：新终端继承的 PATH 可能同样没有 ~/.bun/bin
    let exe_str = omp_exe().to_string_lossy().to_string();
    let omp_ref = if exe_str.contains(' ') {
        format!("\"{exe_str}\"")
    } else {
        exe_str.to_string()
    };
    let cmd = format!("{omp_ref} auth-broker login {provider}");
    launch_login_terminal(&cmd)
}

/// 移除某 provider 的凭据。
#[tauri::command]
pub async fn omp_auth_logout(provider_id: String) -> Result<(), String> {
    if !omp_cli_available() {
        return Err("未检测到 Oh My Pi CLI。".into());
    }
    let provider = provider_id.trim();
    if provider.is_empty() {
        return Err("未指定 OAuth Provider ID。".into());
    }
    let output = run_omp(&["auth-broker", "logout", provider])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// 在独立终端里运行登录命令（TUI 需要真实终端）。
fn launch_login_terminal(command: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // 优先 Windows Terminal，其次回退 cmd 的新窗口
        if Command::new("wt")
            .args(["cmd", "/k", command])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
        Command::new("cmd")
            .args(["/c", "start", "", "cmd", "/k", command])
            .spawn()
            .map_err(|e| format!("启动终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\" to do script \"{}\"",
            command.replace('\\', "\\\\").replace('"', "\\\"")
        );
        Command::new("osascript")
            .args(["-e", &script])
            .spawn()
            .map_err(|e| format!("启动终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let mut c = Command::new(term);
            if term == "gnome-terminal" {
                c.args(["--", "sh", "-c", command]);
            } else {
                c.args(["-e", "sh", "-c", command]);
            }
            if c.spawn().is_ok() {
                return Ok(());
            }
        }
        Err("未找到可用终端模拟器".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = command;
        Err("不支持的操作系统".into())
    }
}

/// 从上游 /v1/models 获取模型列表（由前端用 baseUrl+apiKey 调用通用命令）。
/// 这里保留 OMP 侧的模型目录读取：`omp models --json`。
#[tauri::command]
pub async fn omp_list_models(provider_id: String) -> Result<Vec<OmpModelInfo>, String> {
    if !omp_cli_available() {
        return Ok(vec![]);
    }
    let output = run_omp(&["models", "--json"])?;
    if !output.status.success() {
        return Ok(vec![]);
    }
    let parsed: JsonValue = serde_json::from_slice(&output.stdout).unwrap_or(JsonValue::Null);
    let arr = parsed
        .get("models")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let want = provider_id.trim().to_lowercase();
    let mut out = Vec::new();
    for m in arr {
        let provider = m
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if !want.is_empty() && provider != want {
            continue;
        }
        let Some(id) = m.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        out.push(OmpModelInfo {
            name: m
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_string(),
            api: None,
            reasoning: m.get("reasoning").and_then(|v| v.as_bool()),
            context_window: m.get("contextWindow").and_then(|v| v.as_i64()).unwrap_or(0),
            max_tokens: m.get("maxTokens").and_then(|v| v.as_i64()).unwrap_or(0),
            id: id.to_string(),
        });
    }
    Ok(out)
}

/// 解析密钥形态：`!cmd` / `$(cmd)` → 执行命令取 stdout（secret-bridge）；
/// `${VAR}` / `$VAR` → 读环境变量；其余原样返回（明文密钥）。
fn resolve_secret_form(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(cmd) = trimmed
        .strip_prefix('!')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return run_secret_command(cmd);
    }
    if trimmed.starts_with("$(") && trimmed.ends_with(')') && trimmed.len() > 3 {
        return run_secret_command(&trimmed[2..trimmed.len() - 1]);
    }
    if let Some(var) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        return std::env::var(var.trim()).unwrap_or_default();
    }
    if let Some(var) = trimmed.strip_prefix('$') {
        if !var.is_empty() && !var.contains(char::is_whitespace) {
            return std::env::var(var).unwrap_or_default();
        }
    }
    trimmed.to_string()
}

fn run_secret_command(cmd_str: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", cmd_str]);
        spawn_headless(&mut cmd)
            .output()
            .map(|o| {
                if o.status.success() {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                } else {
                    String::new()
                }
            })
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", cmd_str]);
        match cmd.output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => String::new(),
        }
    }
}

/// 从上游 `/models` 拉取模型目录。apiKey 支持 `$ENV` / `!cmd` 形态
///（models.yml 里密钥常是 secret-bridge 命令，不能当明文直接发请求）。
/// authHeader=true 走 `Authorization: Bearer`，否则走 `X-Api-Key`。
#[tauri::command]
pub async fn omp_fetch_upstream_models(
    base_url: String,
    api_key: String,
    auth_header: Option<bool>,
) -> Result<Vec<OmpModelInfo>, String> {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("Base URL 不能为空。".into());
    }
    let key = resolve_secret_form(&api_key);
    let url = format!("{base}/models");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {e}"))?;
    let mut req = client.get(&url);
    if key.is_empty() {
        // 无密钥：照发（部分中转站目录接口不要求鉴权）
    } else if auth_header.unwrap_or(false) {
        req = req.header("Authorization", format!("Bearer {key}"));
    } else {
        req = req.header("X-Api-Key", &key);
    }
    let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("上游返回 {}", resp.status()));
    }
    let body: JsonValue = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;
    let arr = body
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| body.get("models").and_then(|v| v.as_array()))
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?;
            Some(OmpModelInfo {
                id: id.to_string(),
                name: m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| m.get("display_name").and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(id)
                    .to_string(),
                api: None,
                reasoning: None,
                context_window: 0,
                max_tokens: 0,
            })
        })
        .collect())
}

#[tauri::command]
pub fn get_omp_quota_windows() -> Result<Vec<OmpQuotaWindow>, String> {
    crate::services::session_usage_omp::list_omp_quota_windows().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 与真实 ~/.omp/agent/models.yml 一致的结构（含 secret-bridge apiKey、authHeader、多模型）
    const MODELS_YML: &str = r#"
providers:
    SenseNova:
        baseUrl: https://token.sensenova.cn/v1
        apiKey: '!"C:\secret.exe" --secret-get "credential-x"'
        api: openai-completions
        authHeader: true
        models:
            - id: kimi-k3
              name: kimi-k3
              api: openai-completions
              reasoning: true
              contextWindow: 1000000
              maxTokens: 131072
    Rigel:
        baseUrl: https://sub.flyli.cn/v1
        apiKey: '!"C:\secret.exe" --secret-get "credential-y"'
        api: openai-completions
        authHeader: true
        models:
            - id: grok-4.5
              api: openai-completions
              reasoning: true
"#;

    // 与真实 config.yml 一致：modelRoles 为 "provider/model:level"
    const CONFIG_YML: &str = r#"
defaultThinkingLevel: high
modelRoles:
  default: SenseNova/kimi-k3:high
  smol: SUPER-NB/gpt-5.6-luna:max
  slow: Rigel/grok-4.6:high
  plan: SUPER-NB/gpt-5.6-terra:xhigh
modelProviderOrder:
  - Rigel
  - SenseNova
"#;

    #[test]
    fn parses_real_models_yml() {
        let providers = parse_models_str(MODELS_YML).expect("parse models");
        assert_eq!(providers.len(), 2);

        let sensenova = providers.iter().find(|p| p.id == "SenseNova").unwrap();
        assert_eq!(
            sensenova.base_url.as_deref(),
            Some("https://token.sensenova.cn/v1")
        );
        assert_eq!(sensenova.api.as_deref(), Some("openai-completions"));
        assert!(sensenova.api_key.as_deref().unwrap().contains("secret-get"));
        assert_eq!(sensenova.auth_header, Some(true));
        // 走 https 非本地、非网关 -> api-key/api
        assert_eq!(sensenova.r#type, "api-key");
        assert_eq!(sensenova.models.len(), 1);
        assert_eq!(sensenova.models[0].id, "kimi-k3");
        assert_eq!(sensenova.models[0].context_window, 1000000);
        // name 缺省时回落为 id
        assert_eq!(sensenova.models[0].name, "kimi-k3");
    }

    #[test]
    fn parses_real_config_yml_roles() {
        let roles = parse_roles_str(CONFIG_YML).expect("parse roles");
        assert_eq!(roles.len(), 4);
        let default = roles.iter().find(|r| r.role == "default").unwrap();
        assert_eq!(default.provider_id, "SenseNova");
        assert_eq!(default.model_id, "kimi-k3");
        assert_eq!(default.thinking_level.as_deref(), Some("high"));

        let plan = roles.iter().find(|r| r.role == "plan").unwrap();
        assert_eq!(plan.provider_id, "SUPER-NB");
        assert_eq!(plan.model_id, "gpt-5.6-terra");
        assert_eq!(plan.thinking_level.as_deref(), Some("xhigh"));
    }

    #[test]
    fn role_selector_without_thinking_level() {
        let parsed = parse_role_selector("anthropic/claude-opus-4").unwrap();
        assert_eq!(parsed.0, "anthropic");
        assert_eq!(parsed.1, "claude-opus-4");
        assert!(parsed.2.is_none());
    }

    #[test]
    fn rename_provider_prefix_matches_exact_and_slash() {
        assert_eq!(
            rename_provider_prefix_with("deepseek-api", &legacy_mapping()),
            Some("deepseek".into())
        );
        assert_eq!(
            rename_provider_prefix_with("deepseek-api/deepseek-flash:high", &legacy_mapping()),
            Some("deepseek/deepseek-flash:high".into())
        );
        assert_eq!(
            rename_provider_prefix_with("xai-api", &legacy_mapping()),
            Some("xai".into())
        );
        // 未命中原样返回 None
        assert_eq!(
            rename_provider_prefix_with("Rigel/grok-4.6", &legacy_mapping()),
            None
        );
        assert_eq!(
            rename_provider_prefix_with("deepseek", &legacy_mapping()),
            None
        );
        // 前缀相似但不是旧 id（如 "deepseek-api2"）不改
        assert_eq!(
            rename_provider_prefix_with("deepseek-api2/model", &legacy_mapping()),
            None
        );
    }

    #[test]
    fn renames_models_yaml_provider_keys() {
        let root: YamlValue = serde_yaml::from_str(
            r#"
providers:
    deepseek-api:
        baseUrl: https://api.deepseek.com
        apiKey: sk-test
    Rigel:
        baseUrl: https://sub.flyli.cn/v1
"#,
        )
        .unwrap();
        let (out, changed) = rename_models_yaml_provider_keys_with(&root, &legacy_mapping(), true);
        assert!(changed);
        let providers = out.get("providers").unwrap().as_mapping().unwrap();
        assert!(providers.contains_key(&YamlValue::String("deepseek".into())));
        assert!(!providers.contains_key(&YamlValue::String("deepseek-api".into())));
        assert!(providers.contains_key(&YamlValue::String("Rigel".into())));
        // 改名保留原内容
        let deepseek = providers
            .get(&YamlValue::String("deepseek".into()))
            .unwrap();
        assert_eq!(
            deepseek
                .get(&YamlValue::String("apiKey".into()))
                .and_then(|v| v.as_str()),
            Some("sk-test")
        );
    }

    #[test]
    fn renames_models_yaml_skips_when_target_exists() {
        let root: YamlValue = serde_yaml::from_str(
            r#"
providers:
    deepseek-api:
        baseUrl: https://api.deepseek.com
    deepseek:
        baseUrl: https://builtin
"#,
        )
        .unwrap();
        let (out, changed) = rename_models_yaml_provider_keys_with(&root, &legacy_mapping(), true);
        assert!(changed); // 旧键被清理
        let providers = out.get("providers").unwrap().as_mapping().unwrap();
        // 内置条目不被覆盖
        assert_eq!(
            providers
                .get(&YamlValue::String("deepseek".into()))
                .unwrap()
                .get(&YamlValue::String("baseUrl".into()))
                .and_then(|v| v.as_str()),
            Some("https://builtin")
        );
        assert!(!providers.contains_key(&YamlValue::String("deepseek-api".into())));
    }

    #[test]
    fn renames_config_yaml_refs() {
        let root: YamlValue = serde_yaml::from_str(
            r#"
modelRoles:
    default: deepseek-api/deepseek-flash:high
    slow: Rigel/grok-4.6:high
modelProviderOrder:
    - deepseek-api
    - Rigel
retry:
    fallbackChains:
        deepseek-api/*:
            - deepseek-api/deepseek-flash
            - SenseNova/kimi-k3
"#,
        )
        .unwrap();
        let (out, changed) = rename_config_yaml_provider_refs_with(&root, &legacy_mapping());
        assert!(changed);
        let map = out.as_mapping().unwrap();

        let roles = map
            .get(&YamlValue::String("modelRoles".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(
            roles
                .get(&YamlValue::String("default".into()))
                .and_then(|v| v.as_str()),
            Some("deepseek/deepseek-flash:high")
        );
        assert_eq!(
            roles
                .get(&YamlValue::String("slow".into()))
                .and_then(|v| v.as_str()),
            Some("Rigel/grok-4.6:high")
        );

        let order = map
            .get(&YamlValue::String("modelProviderOrder".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(order[0].as_str(), Some("deepseek"));
        assert_eq!(order[1].as_str(), Some("Rigel"));

        let retry = map
            .get(&YamlValue::String("retry".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        let chains = retry
            .get(&YamlValue::String("fallbackChains".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(chains.contains_key(&YamlValue::String("deepseek/*".into())));
        assert!(!chains.contains_key(&YamlValue::String("deepseek-api/*".into())));
        let chain = chains
            .get(&YamlValue::String("deepseek/*".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(chain[0].as_str(), Some("deepseek/deepseek-flash"));
        assert_eq!(chain[1].as_str(), Some("SenseNova/kimi-k3"));
    }

    #[test]
    fn renames_meta_keys() {
        let mut meta = OmpProviderMetaMap::new();
        let mut entry = OmpProviderMeta::default();
        entry.name = Some("DeepSeek API".into());
        entry.sort_index = Some(0);
        meta.insert("deepseek-api".into(), entry);

        let (out, changed) = rename_meta_keys_with(&meta, &legacy_mapping());
        assert!(changed);
        assert!(out.contains_key("deepseek"));
        assert!(!out.contains_key("deepseek-api"));
        assert_eq!(
            out.get("deepseek").unwrap().name.as_deref(),
            Some("DeepSeek API")
        );
        assert_eq!(out.get("deepseek").unwrap().sort_index, Some(0));

        // 幂等：已迁移的映射再跑一遍无变化
        let (_, changed_again) = rename_meta_keys_with(&out, &legacy_mapping());
        assert!(!changed_again);
    }

    #[test]
    fn synthesizes_library_providers_from_meta_snapshots() {
        let live = vec![sample_provider("Rigel")]; // yml 中已存在
        let mut meta = OmpProviderMetaMap::new();

        // 库条目：已移除、meta 保留完整快照
        let mut removed = sample_provider("SenseNova");
        removed.in_config = false;
        let mut removed_meta = OmpProviderMeta::default();
        removed_meta.name = Some("SenseNova".into());
        removed_meta.sort_index = Some(4);
        removed_meta.config = Some(removed);
        meta.insert("SenseNova".into(), removed_meta);

        // OAuth 快照：不做库化
        let mut oauth_meta = OmpProviderMeta::default();
        let mut oauth_cfg = sample_provider("openai");
        oauth_cfg.r#type = "oauth".into();
        oauth_meta.provider_type = Some("oauth".into());
        oauth_meta.config = Some(oauth_cfg);
        meta.insert("openai".into(), oauth_meta);

        // 无快照的 meta：不合成
        meta.insert("bare".into(), OmpProviderMeta::default());

        let mut providers = live.clone();
        synthesize_library_providers(&mut providers, &meta);

        // live 条目不受影响
        assert_eq!(
            providers
                .iter()
                .find(|p| p.id == "Rigel")
                .unwrap()
                .in_config,
            true
        );
        // 库条目被合成：in_config=false，meta 的名称/排序覆盖生效
        let lib = providers.iter().find(|p| p.id == "SenseNova").unwrap();
        assert_eq!(lib.in_config, false);
        assert_eq!(lib.sort_index, Some(4));
        assert_eq!(lib.name, "SenseNova");
        assert_eq!(lib.api_key.as_deref(), Some("sk-test"));
        // OAuth 与无快照条目不合成
        assert!(!providers.iter().any(|p| p.id == "openai"));
        assert_eq!(providers.len(), 2);
    }

    #[test]
    fn yaml_write_skips_library_entries() {
        let live = sample_provider("Rigel");
        let mut removed = sample_provider("SenseNova");
        removed.in_config = false;
        let yaml = serde_yaml::to_string(&providers_to_yaml_value(&[live, removed])).unwrap();
        assert!(yaml.contains("Rigel"));
        assert!(!yaml.contains("SenseNova"));
    }

    #[test]
    fn ensure_library_snapshot_backfills_missing_config() {
        let mut meta = OmpProviderMetaMap::new();
        // 模拟存量条目：旧版本写入，只有名称/排序，无 config 快照
        let mut legacy = OmpProviderMeta::default();
        legacy.name = Some("Rigel".into());
        legacy.sort_index = Some(3);
        meta.insert("Rigel".into(), legacy);

        let (out, changed) = ensure_library_snapshot(meta, &sample_provider("Rigel"));
        assert!(changed);
        let entry = out.get("Rigel").unwrap();
        assert!(entry.config.is_some());
        let snap = entry.config.as_ref().unwrap();
        assert_eq!(snap.in_config, false);
        assert_eq!(snap.api_key.as_deref(), Some("sk-test"));
        // 既有 sort_index 与名称保留
        assert_eq!(entry.sort_index, Some(3));
        assert_eq!(entry.name.as_deref(), Some("Rigel"));

        // 已有快照时不覆盖（保留 save 写入的最新版）
        let (out2, changed2) = ensure_library_snapshot(out, &sample_provider("Rigel"));
        assert!(!changed2);
        assert_eq!(
            out2.get("Rigel")
                .unwrap()
                .config
                .as_ref()
                .unwrap()
                .in_config,
            false
        );
    }

    #[test]
    fn ensure_library_snapshot_skips_oauth() {
        let mut oauth = sample_provider("openai");
        oauth.r#type = "oauth".into();
        let meta = OmpProviderMetaMap::new();
        let (out, changed) = ensure_library_snapshot(meta, &oauth);
        assert!(!changed);
        assert!(out
            .get("openai")
            .map(|m| m.config.is_none())
            .unwrap_or(true));
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(parse_models_str("").unwrap().is_empty());
        assert!(parse_roles_str("   ").unwrap().is_empty());
    }

    fn sample_provider(id: &str) -> OmpProviderConfig {
        OmpProviderConfig {
            id: id.to_string(),
            name: id.to_string(),
            r#type: "api-key".into(),
            category: "api".into(),
            description: None,
            website_url: None,
            icon: None,
            models: vec![],
            oauth_provider_id: None,
            api: Some("openai-completions".into()),
            base_url: Some("https://example.com/v1".into()),
            api_key: Some("sk-test".into()),
            headers: None,
            auth_header: Some(true),
            raw: None,
            sort_index: None,
            in_config: true,
            usage_script: None,
        }
    }

    #[test]
    fn meta_overlay_overrides_ui_fields() {
        let mut providers = vec![sample_provider("SenseNova")];
        let mut meta: OmpProviderMetaMap = BTreeMap::new();
        meta.insert(
            "SenseNova".into(),
            OmpProviderMeta {
                name: Some("星河 API".into()),
                description: Some("备注".into()),
                website_url: Some("https://example.com".into()),
                icon: Some("openai".into()),
                ..Default::default()
            },
        );
        apply_provider_meta(&mut providers, &meta);

        let p = &providers[0];
        assert_eq!(p.name, "星河 API");
        assert_eq!(p.description.as_deref(), Some("备注"));
        assert_eq!(p.website_url.as_deref(), Some("https://example.com"));
        assert_eq!(p.icon.as_deref(), Some("openai"));
    }

    #[test]
    fn meta_overlay_ignores_empty_name_and_absent_ids() {
        let mut providers = vec![sample_provider("A"), sample_provider("B")];
        let mut meta: OmpProviderMetaMap = BTreeMap::new();
        meta.insert(
            "A".into(),
            OmpProviderMeta {
                name: Some("   ".into()),
                ..Default::default()
            },
        );
        apply_provider_meta(&mut providers, &meta);

        // 空白 name 不覆盖；B 无 meta 条目保持原样
        assert_eq!(providers[0].name, "A");
        assert_eq!(providers[1].name, "B");
    }

    #[test]
    fn save_preserves_provider_order() {
        let mut providers = vec![
            sample_provider("first"),
            sample_provider("second"),
            sample_provider("third"),
        ];
        insert_preserving_order(&mut providers, sample_provider("second"));
        let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["first", "second", "third"]);

        insert_preserving_order(&mut providers, sample_provider("fourth"));
        let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn yaml_write_never_pollutes_models_yml_with_meta_fields() {
        let mut provider = sample_provider("SenseNova");
        provider.description = Some("备注".into());
        provider.website_url = Some("https://example.com".into());
        provider.icon = Some("openai".into());
        let yaml = serde_yaml::to_string(&providers_to_yaml_value(&[provider])).unwrap();

        assert!(yaml.contains("baseUrl"));
        assert!(yaml.contains("apiKey"));
        assert!(!yaml.contains("icon"));
        assert!(!yaml.contains("description"));
        assert!(!yaml.contains("websiteUrl"));
    }

    #[test]
    fn yaml_write_preserves_raw_unknown_keys() {
        // 模拟真实 yml 原生条目：带 omp 自己的 name 字段与未知自定义字段
        let raw_yaml = r#"
baseUrl: https://token.sensenova.cn/v1
apiKey: '!"C:\secret.exe" --secret-get "credential-x"'
api: openai-completions
authHeader: true
name: SenseNova 官方
customFutureField: keep-me
models:
    - id: kimi-k3
      name: kimi-k3
"#;
        let raw: YamlValue = serde_yaml::from_str(raw_yaml).unwrap();
        let mut provider = sample_provider("SenseNova");
        provider.raw = Some(raw);
        provider.base_url = Some("https://new.example.com/v1".into());
        // 编辑态的 apiKey 字段按 overlay 语义覆盖 raw 同名键；真实流程中前端
        // 回传的 apiKey 本就来自 yml 原值，因此 secret-bridge 形态原样写回
        provider.api_key = Some(r#"!"C:\secret.exe" --secret-get "credential-x""#.into());

        let yaml = serde_yaml::to_string(&providers_to_yaml_value(&[provider])).unwrap();
        assert!(yaml.contains("new.example.com"));
        assert!(yaml.contains("customFutureField"));
        assert!(yaml.contains("keep-me"));
        assert!(yaml.contains("SenseNova 官方"));
        // secret-bridge 形态的 apiKey 原样保留
        assert!(yaml.contains("--secret-get"));
    }

    #[test]
    fn strip_provider_references_removes_stale_fallback_and_order() {
        let cfg: YamlValue = serde_yaml::from_str(
            r#"
modelProviderOrder:
  - SenseNova
  - deepseek-api
retry:
  fallbackChains:
    SenseNova/*:
      - SenseNova/sensenova-6.8-flash-lite
    deepseek-api/*:
      - deepseek-api/deepseek-chat
autolearn:
  enabled: true
"#,
        )
        .unwrap();

        let (updated, removed) = strip_provider_references(&cfg, "SenseNova");
        let text = serde_yaml::to_string(&updated).unwrap();

        // 1 个 fallbackChains 通配键 + 1 个 modelProviderOrder 项
        assert_eq!(removed, 2);
        assert!(!text.contains("SenseNova"));
        // 其他供应商的引用与无关配置必须原样保留
        assert!(text.contains("deepseek-api/*"));
        assert!(text.contains("autolearn"));
    }

    #[test]
    fn strip_provider_references_is_noop_when_nothing_matches() {
        let cfg: YamlValue = serde_yaml::from_str(
            r#"
retry:
  fallbackChains:
    deepseek-api/*:
      - deepseek-api/deepseek-chat
"#,
        )
        .unwrap();

        let (updated, removed) = strip_provider_references(&cfg, "Nonexistent");
        assert_eq!(removed, 0);
        assert!(serde_yaml::to_string(&updated)
            .unwrap()
            .contains("deepseek-api/*"));
    }

    #[test]
    fn yaml_write_skips_oauth_providers() {
        let mut oauth = sample_provider("anthropic");
        oauth.r#type = "oauth".into();
        let yaml = serde_yaml::to_string(&providers_to_yaml_value(&[oauth])).unwrap();

        assert!(!yaml.contains("anthropic"));
        // 根结构仍是 providers: {} 空映射
        assert!(yaml.contains("providers"));
    }

    #[test]
    fn oauth_providers_synthesized_from_meta() {
        let mut meta: OmpProviderMetaMap = BTreeMap::new();
        meta.insert(
            "ogg-openai".into(),
            OmpProviderMeta {
                name: Some("OpenAI".into()),
                provider_type: Some("oauth".into()),
                oauth_provider_id: None, // 回退为 id
                ..Default::default()
            },
        );
        meta.insert(
            "SenseNova".into(),
            OmpProviderMeta {
                provider_type: Some("api-key".into()), // 非 oauth 不合成
                ..Default::default()
            },
        );

        let mut providers = vec![sample_provider("SenseNova")];
        synthesize_oauth_providers(&mut providers, &meta);

        assert_eq!(providers.len(), 2);
        let oauth = providers.iter().find(|p| p.id == "ogg-openai").unwrap();
        assert_eq!(oauth.r#type, "oauth");
        assert_eq!(oauth.name, "OpenAI");
        assert_eq!(oauth.oauth_provider_id.as_deref(), Some("ogg-openai"));
        assert!(oauth.models.is_empty());
        assert!(oauth.base_url.is_none());
    }

    #[test]
    fn resolve_secret_form_handles_plain_and_env() {
        // 明文原样
        assert_eq!(resolve_secret_form("sk-abc123"), "sk-abc123");
        // 环境变量形态
        std::env::set_var("OGG_TEST_SECRET_VAR", "resolved-key");
        assert_eq!(resolve_secret_form("$OGG_TEST_SECRET_VAR"), "resolved-key");
        assert_eq!(
            resolve_secret_form("${OGG_TEST_SECRET_VAR}"),
            "resolved-key"
        );
        // 未知变量 → 空（不把 $VAR 字面量当密钥发出去）
        assert_eq!(resolve_secret_form("$OGG_UNSET_VAR_XYZ_42"), "");
    }
    #[test]
    fn looks_like_uuid_detects_uuid_shapes() {
        assert!(looks_like_uuid("a3bab366-ed10-4277-9281-1dd499b4008d"));
        assert!(!looks_like_uuid("优云智算"));
        assert!(!looks_like_uuid("SenseNova"));
        assert!(!looks_like_uuid("short-uuid"));
        assert!(!looks_like_uuid("a3bab366-ed10-4277-9281-1dd499b4008"));
    }

    #[test]
    fn models_rename_drop_on_conflict_controls_semantics() {
        let root: YamlValue = serde_yaml::from_str(
            "providers:\n  old-id:\n    baseUrl: https://a\n  target:\n    baseUrl: https://b",
        )
        .unwrap();
        let mapping = vec![("old-id".to_string(), "target".to_string())];
        // drop_on_conflict = true（legacy 语义）：目标已存在 → 丢弃旧条目（同服务去重）
        let (out, changed) = rename_models_yaml_provider_keys_with(&root, &mapping, true);
        assert!(changed);
        let providers = out.get("providers").unwrap();
        assert!(providers.get("target").is_some());
        assert!(providers.get("old-id").is_none());
        // drop_on_conflict = false（UUID 迁移语义）：目标被占 → 跳过保留旧条目
        let (out, changed) = rename_models_yaml_provider_keys_with(&root, &mapping, false);
        assert!(!changed);
        let providers = out.get("providers").unwrap();
        assert!(providers.get("old-id").is_some());
        assert!(providers.get("target").is_some());
    }

    #[test]
    #[serial_test::serial]
    fn uuid_keys_migrate_to_meta_names() {
        // 临时 HOME 隔离（omp_agent_dir / omp_meta_path 都走 get_home_dir）
        let tmp = tempfile::tempdir().unwrap();
        let old_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", tmp.path());
        (|| {
            let uuid = "a3bab366-ed10-4277-9281-1dd499b4008d";
            let dir = omp_agent_dir();
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                models_path(&dir),
                format!(
                    "providers:\n  {uuid}:\n    baseUrl: https://api.modelverse.cn/v1\n    apiKey: sk-x\n    api: openai-completions\n    models:\n      - id: deepseek-v4.1-flash\n        name: deepseek-v4.1-flash\n        contextWindow: 128000\n        maxTokens: 8192\n"
                ),
            )
            .unwrap();
            fs::write(
                config_path(&dir),
                format!("modelRoles:\n  default: {uuid}/deepseek-v4.1-flash:high\n"),
            )
            .unwrap();
            let mut meta: OmpProviderMetaMap = BTreeMap::new();
            meta.insert(
                uuid.to_string(),
                OmpProviderMeta {
                    name: Some("优云智算".into()),
                    ..Default::default()
                },
            );
            write_provider_meta(&meta).unwrap();

            try_migrate_uuid_provider_keys().unwrap();

            let models_text = fs::read_to_string(models_path(&dir)).unwrap();
            assert!(
                models_text.contains("优云智算"),
                "models.yml should use the display name: {models_text}"
            );
            assert!(!models_text.contains(uuid));
            let config_text = fs::read_to_string(config_path(&dir)).unwrap();
            assert!(
                config_text.contains("优云智算/deepseek-v4.1-flash:high"),
                "role selector should be rewritten: {config_text}"
            );
            let meta_after = read_provider_meta();
            assert!(meta_after.contains_key("优云智算"));
            assert!(!meta_after.contains_key(uuid));
        })();
        match old_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }
}
