use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use rand::RngExt;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    symbols,
    widgets::{LineGauge, Paragraph},
};
use regex::Regex;
use reqwest::{Client, header};
use rodio::{Decoder, DeviceSinkBuilder, Player};
use serde::Deserialize;
use serde_json::Value;
use std::{
    env,
    error::Error,
    io::Cursor,
    io::stdout,
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone)]
struct DanmakuData {
    time: f32,
    text: String,
    color: Color,
}

struct ActiveDanmaku {
    text: String,
    x: f32,
    relative_y: f32,
    speed: f32,
    color: Color,
}

#[derive(Debug, Clone, Deserialize)]
struct SubtitleItem {
    from: f32,
    to: f32,
    content: String,
}

#[derive(Debug, Deserialize)]
struct SubtitleFile {
    body: Vec<SubtitleItem>,
}

fn print_help() {
    println!("bilidm - Bilibili Danmaku Visualizer");
    println!();
    println!("USAGE:");
    println!("    bilidm <BV_ID | URL>");
    println!();
    println!("ARGUMENTS:");
    println!("    <BV_ID | URL>    BV ID (e.g., BV1WYXDB7EPm) or full URL containing BV ID");
    println!();
    println!("EXAMPLES:");
    println!("    bilidm BV1WYXDB7EPm");
    println!("    bilidm https://www.bilibili.com/video/BV1WYXDB7EPm");
    println!();
    println!("KEYBOARD SHORTCUTS:");
    println!("    q / Esc    Exit the player");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || &args[1] == "-h" || &args[1] == "--help" {
        print_help();
        return Ok(());
    }

    let input = &args[1];
    let bvid = extract_bvid(input).ok_or("Failed to find a valid BV ID in the input")?;

    let (mut danmakus, subtitles, audio_bytes) = fetch_data(&bvid).await?;
    danmakus.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());

    if danmakus.is_empty() {
        println!("No danmaku found, possibly due to API rate limiting or no danmaku in the video.");
        return Ok(());
    }

    thread::spawn(move || {
        let handle = DeviceSinkBuilder::open_default_sink().unwrap();
        let player = Player::connect_new(handle.mixer());
        let cursor = Cursor::new(audio_bytes);
        let source = Decoder::try_from(cursor).unwrap();
        player.append(source);
        player.sleep_until_end();
    });

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, danmakus, subtitles);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error occurred: {:?}", err);
    }

    Ok(())
}

fn extract_bvid(input: &str) -> Option<String> {
    let re = Regex::new(r"BV[1-9A-HJ-NP-Za-km-z]{10}").unwrap();
    re.find(input).map(|m| m.as_str().to_string())
}

fn decode_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        if *offset >= data.len() {
            return None;
        }
        let byte = data[*offset];
        *offset += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    Some(result)
}

fn skip_field(wire_type: u64, data: &[u8], offset: &mut usize) -> bool {
    match wire_type {
        0 => decode_varint(data, offset).is_some(),
        1 => {
            *offset += 8;
            *offset <= data.len()
        }
        2 => {
            if let Some(len) = decode_varint(data, offset) {
                *offset += len as usize;
                *offset <= data.len()
            } else {
                false
            }
        }
        5 => {
            *offset += 4;
            *offset <= data.len()
        }
        _ => false,
    }
}

fn parse_seg_so(data: &[u8], danmakus: &mut Vec<DanmakuData>) {
    let mut offset = 0;
    while offset < data.len() {
        let Some(key) = decode_varint(data, &mut offset) else {
            break;
        };
        let wire_type = key & 0x07;
        let field_number = key >> 3;

        if field_number == 1 && wire_type == 2 {
            let Some(len) = decode_varint(data, &mut offset) else {
                break;
            };
            let end = offset + (len as usize);
            if end > data.len() {
                break;
            }

            parse_dm_elem(&data[offset..end], danmakus);
            offset = end;
        } else {
            if !skip_field(wire_type, data, &mut offset) {
                break;
            }
        }
    }
}

fn parse_dm_elem(data: &[u8], danmakus: &mut Vec<DanmakuData>) {
    let mut offset = 0;
    let mut progress = 0;
    let mut color = 16777215;
    let mut content = String::new();

    while offset < data.len() {
        let Some(key) = decode_varint(data, &mut offset) else {
            break;
        };
        let wire_type = key & 0x07;
        let field_number = key >> 3;

        match (field_number, wire_type) {
            (2, 0) => {
                progress = decode_varint(data, &mut offset).unwrap_or(0);
            }
            (5, 0) => {
                color = decode_varint(data, &mut offset).unwrap_or(16777215);
            }
            (7, 2) => {
                let len = decode_varint(data, &mut offset).unwrap_or(0) as usize;
                if offset + len <= data.len() {
                    content = String::from_utf8_lossy(&data[offset..offset + len]).to_string();
                }
                offset += len;
            }
            _ => {
                if !skip_field(wire_type, data, &mut offset) {
                    break;
                }
            }
        }
    }

    if !content.is_empty() {
        danmakus.push(DanmakuData {
            time: (progress as f32) / 1000.0,
            text: content,
            color: Color::Rgb(
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
            ),
        });
    }
}

fn build_http_client(bvid: &str) -> Result<reqwest::Client, Box<dyn Error>> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        "authority",
        header::HeaderValue::from_static("api.bilibili.com"),
    );
    headers.insert("accept", header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"));

    headers.insert(
        header::REFERER,
        format!("https://www.bilibili.com/video/{}", bvid).parse()?,
    );

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
        .default_headers(headers)
        .build()?;
    Ok(client)
}
async fn fetch_danmaku(bvid: &str) -> Result<Vec<DanmakuData>, Box<dyn Error>> {
    let client = build_http_client(bvid)?;

    let info_url = format!(
        "https://api.bilibili.com/x/web-interface/view?bvid={}",
        bvid
    );
    let info_resp: Value = client
        .get(&info_url)
        .header("Referer", "https://www.bilibili.com/")
        .send()
        .await?
        .json()
        .await?;

    if info_resp["code"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!(
            "Failed to fetch video info: {}",
            info_resp["message"].as_str().unwrap_or("Unknown error")
        )
        .into());
    }

    let cid = info_resp["data"]["cid"].as_i64().unwrap();
    let mut danmakus = Vec::new();

    for seg_idx in 1..=2 {
        let dm_url = format!(
            "https://api.bilibili.com/x/v2/dm/web/seg.so?type=1&oid={}&segment_index={}",
            cid, seg_idx
        );
        let dm_bytes = client
            .get(&dm_url)
            .header("Referer", "https://www.bilibili.com/")
            .header("Accept", "application/octet-stream")
            .send()
            .await?
            .bytes()
            .await?;

        if dm_bytes.is_empty() {
            break;
        }
        parse_seg_so(&dm_bytes, &mut danmakus);
    }

    Ok(danmakus)
}
async fn fetch_subtitles(bvid: &str) -> Result<Vec<SubtitleItem>, Box<dyn Error>> {
    let client = build_http_client(bvid)?;

    let info_url = format!(
        "https://api.bilibili.com/x/web-interface/view?bvid={}",
        bvid
    );
    let info_resp: Value = client.get(&info_url).send().await?.json().await?;

    if info_resp["code"].as_i64().unwrap_or(-1) != 0 {
        return Err("Failed to fetch video info".into());
    }

    let aid = info_resp["data"]["aid"].as_i64().unwrap();
    let cid = info_resp["data"]["cid"].as_i64().unwrap();

    let player_url = format!(
        "https://api.bilibili.com/x/player/wbi/v2?aid={}&cid={}",
        aid, cid
    );

    let player_resp: Value = client
        .get(&player_url)
        .header("Referer", "https://www.bilibili.com/")
        .header("Cookie", "")
        .send()
        .await?
        .json()
        .await?;

    let mut subtitle_url = None;

    if player_resp["code"].as_i64().unwrap_or(-1) == 0
        && let Some(subtitles) = player_resp["data"]["subtitle"]["subtitles"].as_array()
    {
        for sub in subtitles {
            let lan = sub["lan"].as_str().unwrap_or("");
            let url = sub["subtitle_url"].as_str().unwrap_or("");
            if !lan.starts_with("ai-") && !url.is_empty() {
                subtitle_url = Some(url.to_string());
                break;
            }
        }

        if subtitle_url.is_none() {
            for sub in subtitles {
                let lan = sub["lan"].as_str().unwrap_or("");
                let url = sub["subtitle_url"].as_str().unwrap_or("");
                if lan.starts_with("ai-") && !url.is_empty() {
                    subtitle_url = Some(url.to_string());
                    break;
                }
            }
        }
    }

    let mut parsed_subtitles = Vec::new();
    if let Some(url) = subtitle_url {
        let full_url = if url.starts_with("//") {
            format!("https:{}", url)
        } else {
            url
        };

        if let Ok(sub_json) = client
            .get(&full_url)
            .send()
            .await?
            .json::<SubtitleFile>()
            .await
        {
            parsed_subtitles = sub_json.body;
        }
    }

    Ok(parsed_subtitles)
}

async fn fetch_audio_data(bvid: &str) -> Result<bytes::Bytes, Box<dyn Error>> {
    let client = build_http_client(bvid)?;
    let info_url = format!(
        "https://api.bilibili.com/x/web-interface/view?bvid={}",
        bvid
    );
    let info_resp: Value = client.get(&info_url).send().await?.json().await?;
    if info_resp["code"].as_i64().unwrap_or(-1) != 0 {
        return Err("Failed to fetch video info".into());
    }
    let cid = info_resp["data"]["cid"].as_i64().unwrap();

    let playurl_url = format!(
        "https://api.bilibili.com/x/player/playurl?bvid={}&cid={}&fnval=16",
        bvid, cid
    );
    let play_resp: Value = client
        .get(&playurl_url)
        .header("Referer", "https://www.bilibili.com/")
        .send()
        .await?
        .json()
        .await?;

    let mut audio_bytes = bytes::Bytes::new();
    if play_resp["code"].as_i64().unwrap_or(-1) == 0 {
        if let Some(audio_arr) = play_resp["data"]["dash"]["audio"].as_array() {
            let mut audios = audio_arr.to_vec();
            audios.sort_by_key(|a| a["id"].as_i64().unwrap_or(0));
            audios.reverse();

            if let Some(best_audio) = audios.first()
                && let Some(audio_url) = best_audio["baseUrl"]
                    .as_str()
                    .or_else(|| best_audio["base_url"].as_str())
            {
                let audio_res = client.get(audio_url).send().await?;

                if audio_res.status().is_success() {
                    audio_bytes = audio_res.bytes().await?;
                } else {
                    eprintln!(
                        "警告：下载音频流失败，CDN 返回状态码 {}",
                        audio_res.status()
                    );
                }
            }
        }
    } else {
        eprintln!(
            "警告：获取 playurl 失败: {}",
            play_resp["message"].as_str().unwrap_or("")
        );
    }
    Ok(audio_bytes)
}

// pub async fn fetch_audio_data(bvid: &str) -> Result<Bytes, Box<dyn Error>> {
//     let client = build_http_client(bvid)?;

//     let mut headers = header::HeaderMap::new();

//     headers.insert(
//         header::REFERER,
//         format!("https://www.bilibili.com/video/{}", bvid).parse()?,
//     );

//     let view_url = format!(
//         "https://api.bilibili.com/x/web-interface/view?bvid={}",
//         bvid
//     );

//     let resp: Response = client
//         .get(&view_url)
//         .headers(headers.clone())
//         .send()
//         .await?;

//     if !resp.status().is_success() {
//         return Err(format!("获取视频信息失败: HTTP {}", resp.status()).into());
//     }

//     let body = resp.text().await?;
//     let json: Value = serde_json::from_str(&body)?;

//     let data = json.get("data").ok_or("API 返回数据无效: data 字段缺失")?;

//     let cid = data
//         .get("pages")
//         .and_then(|p| p.get(0))
//         .and_then(|p| p.get("cid"))
//         .and_then(|c| c.as_i64())
//         .ok_or("无法解析 CID")?;

//     let play_url = format!(
//         "http://api.bilibili.com/x/player/wbi/playurl?fnval=16&bvid={}&cid={}",
//         bvid, cid
//     );

//     let resp: Response = client
//         .get(&play_url)
//         .headers(headers.clone())
//         .send()
//         .await?;

//     if !resp.status().is_success() {
//         return Err(format!("获取播放地址失败: HTTP {}", resp.status()).into());
//     }

//     let body = resp.text().await?;
//     let json: Value = serde_json::from_str(&body)?;

//     let audio_list = json
//         .get("data")
//         .and_then(|d| d.get("dash"))
//         .and_then(|d| d.get("audio"))
//         .and_then(|a| a.as_array())
//         .ok_or("音频字段为空或格式错误")?;

//     if audio_list.is_empty() {
//         return Err("未找到音频流".into());
//     }

//     let base_url = audio_list[0]
//         .get("baseUrl")
//         .and_then(|u| u.as_str())
//         .ok_or("无法解析音频 BaseUrl")?;

//     let audio_resp: Response = client.get(base_url).headers(headers).send().await?;

//     if !audio_resp.status().is_success() {
//         return Err(format!("音频下载失败: HTTP {}", audio_resp.status()).into());
//     }

//     let bytes = audio_resp.bytes().await?;

//     Ok(bytes)
// }

async fn fetch_data(
    bvid: &str,
) -> Result<(Vec<DanmakuData>, Vec<SubtitleItem>, bytes::Bytes), Box<dyn Error>> {
    let danmakus = fetch_danmaku(bvid).await?;
    let subtitles = fetch_subtitles(bvid).await?;
    let audio_bytes = fetch_audio_data(bvid).await?;
    Ok((danmakus, subtitles, audio_bytes))
}
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut pending_danmakus: Vec<DanmakuData>,
    subtitles: Vec<SubtitleItem>,
) -> Result<(), Box<dyn Error>> {
    let mut active_danmakus: Vec<ActiveDanmaku> = Vec::new();
    let start_time = Instant::now();
    let mut rng = rand::rng();

    let total_time = pending_danmakus
        .last()
        .map(|d| d.time)
        .unwrap_or(1.0)
        .max(1.0);
    let time_offset = pending_danmakus.first().map(|d| d.time).unwrap_or(0.0);

    let tick_rate = Duration::from_millis(8);
    let mut last_tick = Instant::now();

    loop {
        let elapsed_sec = start_time.elapsed().as_secs_f32();
        let virtual_time = elapsed_sec + time_offset;

        terminal.draw(|f| {
            let size = f.area();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),    //弹幕
                    Constraint::Length(1), //字幕
                    Constraint::Length(1), //进度条
                ])
                .split(size);

            let danmaku_area = chunks[0];
            let subtitle_area = chunks[1];
            let progress_area = chunks[2];

            let buf = f.buffer_mut();
            for dm in &active_danmakus {
                let mut current_x = dm.x as i32;
                let current_y =
                    danmaku_area.y + (dm.relative_y * danmaku_area.height as f32) as u16;

                for c in dm.text.chars() {
                    let char_width = c.width().unwrap_or(0) as i32;
                    if char_width == 0 {
                        continue;
                    }

                    if current_x >= 0
                        && current_x + char_width <= danmaku_area.width as i32
                        && current_y < danmaku_area.bottom()
                        && let Some(cell) = buf.cell_mut((current_x as u16, current_y))
                    {
                        cell.set_char(c).set_style(Style::default().fg(dm.color));
                    }
                    current_x += char_width;
                }
            }

            let ratio = (virtual_time / total_time) as f64;
            let safe_ratio = ratio.clamp(0.0, 1.0);

            let current_secs = virtual_time as u64;
            let total_secs = total_time as u64;

            let label = format!(
                "{:02}:{:02} / {:02}:{:02}",
                current_secs / 60,
                current_secs % 60,
                total_secs / 60,
                total_secs % 60
            );

            let current_sub = subtitles
                .iter()
                .find(|s| virtual_time >= s.from && virtual_time <= s.to);

            if let Some(sub) = current_sub {
                let paragraph = Paragraph::new(sub.content.as_str())
                    .style(Style::default().fg(Color::LightGreen)) // 稍微给个暗背景突出字幕
                    .alignment(Alignment::Center);

                f.render_widget(paragraph, subtitle_area);
            }

            let bilibili_pink = Color::Rgb(251, 113, 152);
            let gauge = LineGauge::default()
                .filled_style(Style::default().fg(bilibili_pink))
                .style(Style::default().fg(Color::DarkGray))
                .filled_symbol(symbols::line::THICK_HORIZONTAL)
                .unfilled_symbol(symbols::line::THICK_HORIZONTAL)
                .ratio(safe_ratio)
                .label(label);

            f.render_widget(gauge, progress_area);
        })?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && (key.code == KeyCode::Char('q') || key.code == KeyCode::Esc)
        {
            return Ok(());
        }

        if last_tick.elapsed() >= tick_rate {
            let term_width = terminal.size()?.width as f32;

            while !pending_danmakus.is_empty() && pending_danmakus[0].time <= virtual_time {
                let dm_data = pending_danmakus.remove(0);

                active_danmakus.push(ActiveDanmaku {
                    text: dm_data.text,
                    x: term_width,
                    relative_y: rng.random_range(0.0..1.0),
                    speed: rng.random_range(20.0..25.0),
                    color: dm_data.color,
                });
            }

            let dt = last_tick.elapsed().as_secs_f32();
            for dm in &mut active_danmakus {
                dm.x -= dm.speed * dt;
            }

            active_danmakus.retain(|dm| {
                let total_width: usize = dm.text.chars().map(|c| c.width().unwrap_or(0)).sum();
                dm.x + (total_width as f32) > 0.0
            });

            if pending_danmakus.is_empty() && active_danmakus.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(500));
                return Ok(());
            }
            last_tick = Instant::now();
        }
    }
}
