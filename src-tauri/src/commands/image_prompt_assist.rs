//! Prompt polishing for the image workbench: one request through the local
//! router to any text model a relay, account or aggregate offers.

use super::image::{paths, MAX_PROMPT_CHARS};
use super::image_prompts::{read_library, LibraryKind, PromptGroup};
use crate::core::{
    gateway, models_dev, repository::Repository, router, router_proxy, routing_runtime, storage,
};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::State;

const SETTINGS_FILE: &str = "image-prompt-assist.json";
const MAX_INSTRUCTION_CHARS: usize = 8_000;
const MAX_REQUEST_CHARS: usize = 500;
const MAX_MODEL_ID_CHARS: usize = 300;
/// Material categories sent along; past this the user is asked to tick fewer.
const MAX_MATERIAL_CHARS: usize = 24_000;
const MAX_MATERIAL_IDS: usize = 100;
const MAX_MATERIAL_ID_CHARS: usize = 64;
const MAX_OUTPUT_TOKENS: u64 = 4096;
/// The workbench sends a downscaled copy; this only stops something far larger.
const MAX_REFERENCE_CHARS: usize = 8 * 1024 * 1024;
const REFERENCE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];
/// Reasoning models can think for a while before they answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Image models cannot rewrite text, so they are not offered.
const IMAGE_MODEL_MARKERS: [&str; 2] = ["image", "dall"];

pub const DEFAULT_INSTRUCTION: &str = "你是 GPT Image 生图提示词优化助手。把用户给出的画面描述改写成一段可直接用于生图的提示词：
- 保留原意和所有明确要求（主体、数量、画面中的文字、比例、风格），不要擅自更换主题或增加敏感内容。
- 补全画面要素：主体的外观与动作、场景与环境、构图与视角、光线、色彩、材质与质感、画风或媒介、整体氛围。
- 画面中需要出现的文字用引号原样保留。
- 使用与用户输入相同的语言，写成连贯的描述，不要列表、标题或解释。
- 只输出优化后的提示词本身。";

/// What the user chose: the model last used, their own instruction ("" for the default),
/// whether the reference image is left out of polishing, and the material categories
/// ticked to go along (none until the user ticks one: materials cost tokens).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistSettings {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub skip_reference: bool,
    #[serde(default)]
    pub materials: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistModel {
    pub id: String,
    pub label: String,
    pub source_name: String,
    pub kind: gateway::ModelKind,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistState {
    pub settings: AssistSettings,
    pub default_instruction: &'static str,
    pub models: Vec<AssistModel>,
}

fn settings_path(paths: &CodexPaths) -> std::path::PathBuf {
    paths.app_data_dir.join(SETTINGS_FILE)
}

fn read_settings(paths: &CodexPaths) -> AssistSettings {
    std::fs::read(settings_path(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_settings(paths: &CodexPaths, settings: AssistSettings) -> Result<AssistSettings, String> {
    let settings = AssistSettings {
        model: settings.model.trim().to_owned(),
        instruction: settings.instruction.trim().to_owned(),
        skip_reference: settings.skip_reference,
        materials: settings.materials,
    };
    if settings.model.chars().count() > MAX_MODEL_ID_CHARS
        || settings.instruction.chars().count() > MAX_INSTRUCTION_CHARS
        || !material_ids_fit(&settings.materials)
    {
        return Err("image.errors.assistSettings".into());
    }
    let contents = serde_json::to_vec_pretty(&settings)
        .map_err(|_| "image.errors.assistSettings".to_owned())?;
    storage::write_atomic(&settings_path(paths), &contents)
        .map_err(|_| "image.errors.assistSettings".to_owned())?;
    Ok(settings)
}

fn material_ids_fit(ids: &[String]) -> bool {
    ids.len() <= MAX_MATERIAL_IDS
        && ids
            .iter()
            .all(|id| id.chars().count() <= MAX_MATERIAL_ID_CHARS)
}

/// The ticked material categories, in the library's order, as the model reads them;
/// "" when none is ticked or the ticked ones are empty.
fn materials_text(groups: &[PromptGroup], ids: &[String]) -> Result<String, String> {
    let mut text = String::new();
    for group in groups.iter().filter(|group| ids.contains(&group.id)) {
        if group.templates.is_empty() {
            continue;
        }
        text.push_str(&format!("## {}\n", group.name));
        for item in &group.templates {
            let content = item.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
            match item.note.as_str() {
                "" => text.push_str(&format!("- {}：{content}\n", item.title)),
                note => text.push_str(&format!("- {}（{note}）：{content}\n", item.title)),
            }
        }
    }
    if text.chars().count() > MAX_MATERIAL_CHARS {
        return Err("image.errors.assistMaterials".into());
    }
    Ok(text)
}

fn is_image_model(model_id: &str) -> bool {
    let lower = model_id.to_ascii_lowercase();
    IMAGE_MODEL_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Every text model the router can reach, as the gateway names them.
fn text_models(store: &router::Store, catalog: &models_dev::Catalog) -> Vec<AssistModel> {
    gateway::all_models(store, catalog)
        .into_iter()
        .filter(|model| !is_image_model(&model.model_id))
        .map(|model| AssistModel {
            id: model.id,
            label: model.label,
            source_name: model.source_name,
            kind: model.kind,
        })
        .collect()
}

fn load_state(paths: &CodexPaths) -> Result<AssistState, String> {
    let store = router::load_store(paths)?;
    let catalog = models_dev::load(&paths.app_data_dir);
    Ok(AssistState {
        settings: read_settings(paths),
        default_instruction: DEFAULT_INSTRUCTION,
        models: text_models(&store, &catalog),
    })
}

/// The user turn's text: the prompt to rewrite, the materials to draw on, and the
/// one-off requirement if any. With a reference image the prompt becomes an edit
/// of it, as image-to-image expects.
fn user_message(prompt: &str, request: &str, materials: &str, with_reference: bool) -> String {
    let lead = if with_reference {
        "附图是这次图生图的底本。请对照底本优化下面的生图提示词，写成改图指令：\
         说明要保留底本的哪些部分（如主体、构图、画风），以及要改成什么样；\
         不要重复描述底本里不变的细节，也不要和底本的实际内容矛盾。"
    } else {
        "请优化下面的生图提示词："
    };
    let mut message = format!("{lead}\n<prompt>\n{}\n</prompt>", prompt.trim());
    if !materials.is_empty() {
        message.push_str(&format!(
            "\n下面是用户的素材库（服装、动作、视角等），按分类列出。\
             根据画面需要从中选用：每个分类最多取一两条，条目列出多个单品时只取需要的，自然地写进提示词；\
             提示词已经写明的不要替换，与画面不搭的分类就不用，不要堆砌，也不要提到素材库。\
             写服装时按外套、上衣、下装、袜子、鞋、配饰的顺序，一套穿搭的亮点不超过三个，配色保持统一。\
             \n<materials>\n{materials}</materials>"
        ));
    }
    if !request.trim().is_empty() {
        message.push_str(&format!("\n这次的额外要求：{}", request.trim()));
    }
    message
}

/// The reference image as a Messages image block; only a base64 data URL is accepted.
fn reference_block(data_url: &str) -> Result<Value, String> {
    let (media_type, data) = data_url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .filter(|(media_type, data)| {
            REFERENCE_TYPES.contains(media_type)
                && !data.is_empty()
                && data.len() <= MAX_REFERENCE_CHARS
        })
        .ok_or("image.errors.assistReference")?;
    Ok(json!({
        "type": "image",
        "source": { "type": "base64", "media_type": media_type, "data": data },
    }))
}

/// The user turn: the text alone, or the reference image before it.
fn user_content(
    prompt: &str,
    request: &str,
    materials: &str,
    reference: Option<&str>,
) -> Result<Value, String> {
    let Some(reference) = reference else {
        return Ok(json!(user_message(prompt, request, materials, false)));
    };
    Ok(json!([
        reference_block(reference)?,
        { "type": "text", "text": user_message(prompt, request, materials, true) },
    ]))
}

/// The text of a Messages reply, without a code fence a model may wrap it in.
fn reply_text(reply: &Value) -> String {
    let text: String = reply["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    let text = text.trim();
    let unfenced = text
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .map(|inner| inner.split_once('\n').map_or(inner, |(_, body)| body))
        .unwrap_or(text);
    unfenced.trim().to_owned()
}

fn check_input(model: &str, prompt: &str, request: &str) -> Result<(), String> {
    let prompt_chars = prompt.trim().chars().count();
    if prompt_chars == 0 || prompt_chars > MAX_PROMPT_CHARS {
        return Err("image.errors.prompt".into());
    }
    if model.trim().is_empty() || model.chars().count() > MAX_MODEL_ID_CHARS {
        return Err("image.errors.assistModel".into());
    }
    if request.chars().count() > MAX_REQUEST_CHARS {
        return Err("image.errors.assistRequest".into());
    }
    Ok(())
}

async fn polish(
    paths: CodexPaths,
    model: String,
    prompt: String,
    request: String,
    materials: Vec<String>,
    reference: Option<String>,
) -> Result<String, String> {
    check_input(&model, &prompt, &request)?;
    if !material_ids_fit(&materials) {
        return Err("image.errors.assistMaterials".into());
    }
    let materials = match materials.is_empty() {
        true => String::new(),
        false => {
            let paths = paths.clone();
            let library =
                super::blocking(move || read_library(&paths, LibraryKind::Materials)).await?;
            materials_text(&library.groups, &materials)?
        }
    };
    let content = user_content(&prompt, &request, &materials, reference.as_deref())?;
    {
        let _guard = routing_runtime::mutation_guard().await;
        routing_runtime::ensure_available(&paths).await?;
    }
    let port = router_proxy::running_port().ok_or("image.errors.assistRouter")?;
    let token = router::load_store(&paths)?.router.bearer_token;
    let settings = read_settings(&paths);
    let instruction = match settings.instruction.trim() {
        "" => DEFAULT_INSTRUCTION,
        custom => custom,
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| "image.errors.network".to_owned())?;
    // `/clients/image-workbench` books the usage to the workbench.
    let response = client
        .post(format!(
            "http://127.0.0.1:{port}/clients/image-workbench/mix/v1/messages"
        ))
        .header("x-api-key", token)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model.trim(),
            "max_tokens": MAX_OUTPUT_TOKENS,
            "system": instruction,
            "messages": [{ "role": "user", "content": content }],
        }))
        .send()
        .await
        .map_err(|error| {
            let key = if error.is_timeout() {
                "image.errors.timeout"
            } else {
                "image.errors.network"
            };
            key.to_owned()
        })?;
    let status = response.status();
    let reply: Value = response
        .json()
        .await
        .map_err(|_| "image.errors.response".to_owned())?;
    if !status.is_success() {
        // The router's and the upstream's own words say more than a generic key.
        return Err(reply["error"]["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("HTTP {status}")));
    }
    let text = reply_text(&reply);
    if text.is_empty() {
        return Err("image.errors.assistEmpty".into());
    }
    Ok(text)
}

#[tauri::command]
pub async fn load_prompt_assist(repo: State<'_, Mutex<Repository>>) -> Result<AssistState, String> {
    let paths = paths(&repo)?;
    super::blocking(move || load_state(&paths)).await
}

#[tauri::command]
pub async fn save_prompt_assist_settings(
    repo: State<'_, Mutex<Repository>>,
    settings: AssistSettings,
) -> Result<AssistSettings, String> {
    let paths = paths(&repo)?;
    super::blocking(move || write_settings(&paths, settings)).await
}

#[tauri::command]
pub async fn optimize_image_prompt(
    repo: State<'_, Mutex<Repository>>,
    model: String,
    prompt: String,
    request: String,
    materials: Option<Vec<String>>,
    reference: Option<String>,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    polish(
        paths,
        model,
        prompt,
        request,
        materials.unwrap_or_default(),
        reference,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_trimmed_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        assert_eq!(read_settings(&paths), AssistSettings::default());
        let saved = write_settings(
            &paths,
            AssistSettings {
                model: " relay/glm-5 ".into(),
                instruction: "  只输出英文  ".into(),
                skip_reference: true,
                materials: vec!["seed-pose".into()],
            },
        )
        .unwrap();
        assert_eq!(
            saved,
            AssistSettings {
                model: "relay/glm-5".into(),
                instruction: "只输出英文".into(),
                skip_reference: true,
                materials: vec!["seed-pose".into()],
            }
        );
        assert_eq!(read_settings(&paths), saved);
        let long = AssistSettings {
            instruction: "长".repeat(MAX_INSTRUCTION_CHARS + 1),
            ..Default::default()
        };
        assert_eq!(
            write_settings(&paths, long).unwrap_err(),
            "image.errors.assistSettings"
        );
    }

    #[test]
    fn image_models_are_not_offered_for_rewriting() {
        assert!(is_image_model("gpt-image-2.5-flare"));
        assert!(is_image_model("DALL-E-3"));
        assert!(is_image_model("gemini-2.5-flash-image"));
        assert!(!is_image_model("claude-sonnet-5-5"));
        assert!(!is_image_model("glm-5.3"));
    }

    #[test]
    fn the_user_turn_carries_the_prompt_and_a_one_off_requirement() {
        assert_eq!(
            user_message("  雨夜橘猫 ", "", "", false),
            "请优化下面的生图提示词：\n<prompt>\n雨夜橘猫\n</prompt>"
        );
        assert!(user_message("雨夜橘猫", " 赛博朋克 ", "", false)
            .ends_with("\n这次的额外要求：赛博朋克"));
    }

    #[test]
    fn every_seeded_material_fits_one_polish() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("codex"));
        let seeded = read_library(&paths, LibraryKind::Materials).unwrap().groups;
        let all: Vec<String> = seeded.iter().map(|group| group.id.clone()).collect();
        assert!(material_ids_fit(&all));
        let text = materials_text(&seeded, &all).unwrap();
        assert!(text.starts_with("## 服装\n- "), "{}", &text[..40]);
    }

    #[test]
    fn ticked_materials_go_along_in_library_order() {
        use super::super::image_prompts::PromptTemplate;
        let item = |title: &str, note: &str, prompt: &str| PromptTemplate {
            id: title.into(),
            title: title.into(),
            note: note.into(),
            prompt: prompt.into(),
            pinned: false,
        };
        let groups = vec![
            PromptGroup {
                id: "outfit".into(),
                name: "服装".into(),
                templates: vec![item("JK 制服", "日系校园", "水手服，\n百褶裙")],
            },
            PromptGroup {
                id: "pose".into(),
                name: "动作".into(),
                templates: vec![item("回眸", "", "回头望向镜头")],
            },
            PromptGroup {
                id: "empty".into(),
                name: "视角".into(),
                templates: Vec::new(),
            },
        ];
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(
            materials_text(&groups, &ids(&["pose", "empty", "outfit"])).unwrap(),
            "## 服装\n- JK 制服（日系校园）：水手服， 百褶裙\n## 动作\n- 回眸：回头望向镜头\n"
        );
        assert_eq!(
            materials_text(&groups, &ids(&["empty", "gone"])).unwrap(),
            ""
        );
        let long = vec![PromptGroup {
            id: "long".into(),
            name: "长".into(),
            templates: vec![item("长", "", &"长".repeat(MAX_MATERIAL_CHARS))],
        }];
        assert_eq!(
            materials_text(&long, &ids(&["long"])).unwrap_err(),
            "image.errors.assistMaterials"
        );

        let text = user_message("雨夜少女", "赛博朋克", "## 动作\n- 回眸：回头\n", false);
        let materials = text
            .find("<materials>\n## 动作\n- 回眸：回头\n</materials>")
            .unwrap();
        assert!(text.find("</prompt>").unwrap() < materials);
        assert!(text.ends_with("\n这次的额外要求：赛博朋克"), "{text}");
        assert!(!user_message("猫", "", "", false).contains("<materials>"));
    }

    #[test]
    fn a_reference_goes_before_the_text_and_asks_for_an_edit() {
        assert_eq!(
            user_content("猫", "", "", None).unwrap(),
            json!(user_message("猫", "", "", false))
        );
        let content =
            user_content("把猫换成狗", "", "", Some("data:image/jpeg;base64,/9j/")).unwrap();
        assert_eq!(
            content[0],
            json!({ "type": "image", "source": {
                "type": "base64", "media_type": "image/jpeg", "data": "/9j/" } })
        );
        let text = content[1]["text"].as_str().unwrap();
        assert!(text.starts_with("附图是这次图生图的底本"), "{text}");
        assert!(text.contains("<prompt>\n把猫换成狗\n</prompt>"));
        for bad in [
            "https://example.test/cat.png",
            "data:image/svg+xml;base64,PHN2Zz4=",
            "data:image/png;base64,",
        ] {
            assert_eq!(
                user_content("猫", "", "", Some(bad)).unwrap_err(),
                "image.errors.assistReference",
                "{bad}"
            );
        }
    }

    #[test]
    fn reply_text_joins_text_blocks_and_drops_a_fence() {
        let reply = json!({ "content": [
            { "type": "thinking", "thinking": "..." },
            { "type": "text", "text": "```text\n雨夜霓虹街头的橘猫，" },
            { "type": "text", "text": "电影感光影\n```" },
        ]});
        assert_eq!(reply_text(&reply), "雨夜霓虹街头的橘猫，电影感光影");
        assert_eq!(reply_text(&json!({ "content": [] })), "");
    }

    #[test]
    fn input_is_checked_before_any_request() {
        assert_eq!(
            check_input("relay/m", "  ", "").unwrap_err(),
            "image.errors.prompt"
        );
        assert_eq!(
            check_input(" ", "猫", "").unwrap_err(),
            "image.errors.assistModel"
        );
        assert_eq!(
            check_input("relay/m", "猫", &"长".repeat(MAX_REQUEST_CHARS + 1)).unwrap_err(),
            "image.errors.assistRequest"
        );
        assert!(check_input("relay/m", "猫", "赛博朋克").is_ok());
    }
}
