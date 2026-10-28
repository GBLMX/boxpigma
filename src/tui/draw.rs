//! Terminal layout for backend snapshots.
//!
//! Nothing here calls the backend: rows, gauges and overlays are built from
//! [`View`] plus the renderer's cursors and hit map.

use ferriso_tui::art;
use ferriso_tui::ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Fill, Gauge, Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    backend::{Page, SearchKind, View, current_lyric_index, translation_at},
    state::{ContentState, LoginField, LoginMethod},
    utils::format_duration,
};

use super::{
    Focus, Hit, SETTINGS, Tui, artist_count, ensure_visible, field_value, login_fields, palette,
};

pub(super) fn draw(tui: &mut Tui, view: &View, frame: &mut Frame) {
    let palette = palette(view);
    let area = frame.area();
    tui.areas = Default::default();
    frame.render_widget(
        Fill::new(" ").style(Style::default().bg(palette.background)),
        area,
    );
    let footer = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: u16::from(area.height > 0),
        ..area
    };
    let shell_area = Rect {
        height: area.height.saturating_sub(footer.height),
        ..area
    };

    let mut panes = view.panes.clone();
    if let Some((pane, size)) = tui.pane_preview {
        panes.set_size(pane, size);
    }
    let shell = crate::layout::shell(shell_area, &panes, view.nav_position);
    tui.shell_area = shell_area;
    tui.pane_dividers = shell.dividers;

    if !shell.topbar.is_empty() {
        draw_top(tui, view, &palette, shell.topbar, frame);
    }
    draw_main(tui, view, &palette, &shell, frame);
    if !shell.playerbar.is_empty() {
        draw_player(tui, view, &palette, shell.playerbar, frame);
    }
    draw_footer(tui, view, &palette, footer, frame);

    if tui.help {
        draw_help(&palette, area, frame);
    }
}

fn art_time(tui: &Tui) -> f32 {
    tui.tick as f32 * 0.08
}

fn art_palette(p: &super::theme::Palette) -> art::Palette {
    art::Palette {
        abyss: p.background,
        glow: p.surface,
        accent: p.accent,
        highlight: p.text,
    }
}

fn draw_top(tui: &mut Tui, view: &View, p: &super::theme::Palette, area: Rect, frame: &mut Frame) {
    let heading = page_heading(view);
    let mut title = Line::from(vec![
        Span::styled(" boxpigma ", p.title()),
        Span::styled("· ", p.hint()),
        Span::styled(
            heading,
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        ),
    ]);
    if let Some(subtitle) = view.subtitle.as_deref().filter(|s| !s.trim().is_empty()) {
        title
            .spans
            .push(Span::styled(format!("  {subtitle}"), p.hint()));
    }
    if view.loading {
        title.spans.push(Span::styled(
            format!(
                "  {} 加载中…",
                tui.symbols.spinner_activity[(tui.tick as usize
                    / tui.symbols.spinner_ticks_per_frame.max(1) as usize)
                    % tui.symbols.spinner_activity.len()]
            ),
            Style::default().fg(p.warning),
        ));
    }
    frame.render_widget(
        Paragraph::new(title).style(p.panel()),
        Rect { height: 1, ..area },
    );

    let prefix = " 搜索 ";
    let query = if tui.search_editing {
        tui.search.as_str()
    } else {
        view.search.query.as_str()
    };
    let kind = view.search.kind;
    let mut kinds = Vec::new();
    for item in SearchKind::ALL {
        kinds.push(Span::styled(
            format!(" {} ", item.label()),
            if item == kind {
                p.subtle_selected()
            } else {
                p.hint()
            },
        ));
    }
    let mut search_spans = vec![Span::styled(prefix, p.hint())];
    search_spans.push(Span::styled(
        query.to_string(),
        if tui.search_editing {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text)
        },
    ));
    if !query.is_empty() {
        search_spans.push(Span::raw("  "));
    }
    search_spans.extend(kinds);
    let search_line = Line::from(search_spans);
    let search_y = area.y + 1;
    frame.render_widget(
        Paragraph::new(search_line),
        Rect {
            y: search_y,
            height: 1,
            ..area
        },
    );
    if tui.search_editing {
        let before = &query[..char_boundary(query, tui.search_cursor)];
        let x = area.x
            + 1
            + UnicodeWidthStr::width(prefix) as u16
            + UnicodeWidthStr::width(before) as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), search_y));
    }

    let status = if let Some((text, error)) = tui.visible_banner() {
        Line::from(Span::styled(
            format!(" {text}"),
            if error {
                Style::default().fg(p.error)
            } else {
                Style::default().fg(p.muted)
            },
        ))
    } else if let Some(user) = &view.user {
        Line::from(Span::styled(
            format!(" 已登录 · {}", user.nickname),
            Style::default().fg(p.muted),
        ))
    } else {
        Line::from(Span::styled(
            " 未登录 · 搜索与公开榜单可用",
            Style::default().fg(p.muted),
        ))
    };
    frame.render_widget(
        Paragraph::new(status).style(p.panel()),
        Rect {
            y: area.y + 2,
            height: 1,
            ..area
        },
    );
}

fn draw_main(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    shell: &crate::layout::ShellLayout,
    frame: &mut Frame,
) {
    let nav = if shell.sidebar.is_empty() {
        shell.nav
    } else {
        shell.sidebar
    };
    if !nav.is_empty() {
        draw_nav(tui, view, p, nav, frame);
    }
    draw_content(tui, view, p, shell.content, frame);
}

fn draw_nav(tui: &mut Tui, view: &View, p: &super::theme::Palette, area: Rect, frame: &mut Frame) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(p.border))
        .title(Span::styled(" 导航 ", p.title()))
        .style(p.panel());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    struct Row {
        text: String,
        style: Style,
        hit: Option<Hit>,
    }

    let mut nav_rows = Vec::new();
    let mut selected_nav = 0usize;
    for (section_index, section) in view.nav.sections.iter().enumerate() {
        nav_rows.push(Row {
            text: format!(" {}", section.title),
            style: p.header(),
            hit: None,
        });
        for (item_index, item) in section.items.iter().enumerate() {
            let selected = view.nav.section == section_index && view.nav.item == item_index;
            if selected {
                selected_nav = nav_rows.len();
            }
            nav_rows.push(Row {
                text: if selected {
                    format!(
                        "{}{}{}",
                        tui.symbols.nav_capsule_left, item.name, tui.symbols.nav_capsule_right
                    )
                } else {
                    format!("  {}", item.name)
                },
                style: if selected {
                    if tui.focus == Focus::Nav {
                        p.selected()
                    } else {
                        p.subtle_selected()
                    }
                } else {
                    Style::default().fg(p.text)
                },
                hit: Some(Hit::Nav {
                    section: section_index,
                    item: item_index,
                }),
            });
        }
    }

    nav_rows.push(Row {
        text: String::new(),
        style: p.hint(),
        hit: None,
    });
    for (page, label) in [
        (Page::Browse, "浏览"),
        (Page::Queue, "播放队列"),
        (Page::Lyrics, "歌词"),
        (Page::Login, "登录 / 账号"),
        (Page::Settings, "设置"),
    ] {
        let selected = view.page == page;
        if selected {
            selected_nav = nav_rows.len();
        }
        nav_rows.push(Row {
            text: format!("  {label}"),
            style: if selected {
                p.subtle_selected()
            } else {
                Style::default().fg(p.text)
            },
            hit: Some(Hit::Page(page)),
        });
    }
    nav_rows.push(Row {
        text: String::new(),
        style: p.hint(),
        hit: None,
    });
    nav_rows.push(Row {
        text: match &view.user {
            Some(user) => format!(" 已登录 · {}", user.nickname),
            None => " 未登录".into(),
        },
        style: p.hint(),
        hit: None,
    });

    let visible = inner.height as usize;
    tui.nav_rows = visible;
    if visible == 0 {
        return;
    }
    ensure_visible(selected_nav, &mut tui.nav_offset, visible);
    let offset = tui.nav_offset.min(nav_rows.len().saturating_sub(1));

    for (row_index, row) in nav_rows.iter().skip(offset).take(visible).enumerate() {
        let rect = Rect {
            x: inner.x,
            y: inner.y + row_index as u16,
            width: inner.width,
            height: 1,
        };
        let text = fit(&row.text, inner.width as usize);
        frame.render_widget(Paragraph::new(text).style(row.style), rect);
        if let Some(hit) = row.hit {
            tui.areas.hits.push((rect, hit));
        }
    }
}

fn draw_content(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(p.border))
        .title(Span::styled(format!(" {} ", page_heading(view)), p.title()))
        .style(p.panel());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    match view.page {
        Page::Browse => draw_browse(tui, view, p, inner, frame),
        Page::Artist => draw_artist(tui, view, p, inner, frame),
        Page::Queue => draw_queue(tui, view, p, inner, frame),
        Page::Lyrics => draw_lyrics(tui, view, p, inner, frame),
        Page::Login => draw_login(tui, view, p, inner, frame),
        Page::Settings => draw_settings(tui, view, p, inner, frame),
    }
}

fn draw_browse(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    if area.height == 0 {
        return;
    }
    let header = match &view.content {
        ContentState::Songs(_) => Some(song_header(area.width)),
        ContentState::SongLists(_) => Some(list_header(area.width)),
        ContentState::TopLists(_) => Some(list_header(area.width)),
        ContentState::Singers(_) => Some("  歌手".to_string()),
        ContentState::HotSearch(_) => Some("  热搜".to_string()),
        ContentState::Empty | ContentState::Loading | ContentState::Error(_) => None,
    };
    let header_height = u16::from(header.is_some());
    if let Some(header) = header {
        frame.render_widget(
            Paragraph::new(fit(&header, area.width as usize)).style(p.header()),
            Rect { height: 1, ..area },
        );
    }
    let body = Rect {
        y: area.y + header_height,
        height: area.height - header_height,
        ..area
    };
    if body.height == 0 {
        return;
    }
    tui.content_rows = body.height as usize;
    tui.areas.content = body;
    tui.areas.content_offset = tui.content_offset;

    let len = view.content.len();
    let more = view
        .pagination
        .as_ref()
        .is_some_and(|pagination| pagination.has_more);
    let total = len + usize::from(more);
    tui.content_selected = tui.content_selected.min(total.saturating_sub(1));
    ensure_visible(
        tui.content_selected,
        &mut tui.content_offset,
        body.height as usize,
    );
    tui.areas.content_offset = tui.content_offset;

    if total == 0 {
        draw_empty(tui, view, &view.content, p, body, frame);
        return;
    }

    let offset = tui.content_offset;
    for (row_index, content_index) in (offset..total).take(body.height as usize).enumerate() {
        let selected = content_index == tui.content_selected;
        let line = browse_line(view, content_index, area.width as usize, selected, p);
        let rect = Rect {
            y: body.y + row_index as u16,
            height: 1,
            ..body
        };
        frame.render_widget(Paragraph::new(line), rect);
        tui.areas.hits.push((rect, Hit::Content(content_index)));
    }
}

fn browse_line(
    view: &View,
    index: usize,
    width: usize,
    selected: bool,
    p: &super::theme::Palette,
) -> Line<'static> {
    let (text, current) = match &view.content {
        ContentState::Songs(songs) => match songs.get(index) {
            Some(song) => {
                let current = view.player.song.as_ref().map(|s| s.id) == Some(song.id);
                (
                    song_line(
                        index,
                        &song.name,
                        &song.singer,
                        &song.album,
                        song.duration,
                        current,
                        width,
                    ),
                    current,
                )
            }
            None => ("   … 加载更多（o）".to_string(), false),
        },
        ContentState::SongLists(lists) => match lists.get(index) {
            Some(list) => (
                format!(
                    " {:<3} {}  {}",
                    index + 1,
                    fit(&list.name, width.saturating_sub(20).max(8)),
                    list.author
                ),
                false,
            ),
            None => ("   … 加载更多（o）".into(), false),
        },
        ContentState::TopLists(lists) => match lists.get(index) {
            Some(list) => (
                format!(
                    " {:<3} {}  {}",
                    index + 1,
                    fit(&list.name, width.saturating_sub(20).max(8)),
                    list.update
                ),
                false,
            ),
            None => ("   … 加载更多（o）".into(), false),
        },
        ContentState::Singers(singers) => match singers.get(index) {
            Some(singer) => (format!(" {:<3} {}", index + 1, singer.name), false),
            None => ("   … 加载更多（o）".into(), false),
        },
        ContentState::HotSearch(keywords) => match keywords.0.get(index) {
            Some(keyword) => (format!(" {:<3} {}", index + 1, keyword), false),
            None => ("   … 加载更多（o）".into(), false),
        },
        ContentState::Empty | ContentState::Loading | ContentState::Error(_) => {
            (String::new(), false)
        }
    };
    let style = if selected {
        p.selected()
    } else if current {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.text)
    };
    Line::from(Span::styled(fit(&text, width), style))
}

fn draw_artist(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let Some(artist) = view.artist.as_ref() else {
        draw_empty(tui, view, &ContentState::Loading, p, area, frame);
        return;
    };
    let header = Rect {
        height: 2.min(area.height),
        ..area
    };
    let alias = if artist.detail.alias.is_empty() {
        String::new()
    } else {
        format!(" / {}", artist.detail.alias.join(" / "))
    };
    let profile = Line::from(vec![
        Span::styled(format!(" {}", artist.detail.name), p.title()),
        Span::styled(alias, p.hint()),
        Span::styled(
            format!(
                "   热门 {} · 专辑 {} · 相似 {}",
                artist.detail.hot_songs.len(),
                artist.albums.len(),
                artist.similar.len()
            ),
            p.hint(),
        ),
    ]);
    let bio = Line::from(Span::styled(
        format!(
            " {}",
            truncate(
                &artist.detail.brief_desc,
                area.width.saturating_sub(2) as usize
            )
        ),
        p.hint(),
    ));
    frame.render_widget(Paragraph::new(vec![profile, bio]), header);

    let tab_y = area.y + header.height;
    let mut x = area.x;
    for (index, label) in ["热门歌曲", "专辑", "相似歌手"].iter().enumerate() {
        let width = UnicodeWidthStr::width(format!(" {label} ").as_str()) as u16;
        let rect = Rect {
            x,
            y: tab_y,
            width,
            height: 1,
        };
        if rect.right() > area.right() {
            break;
        }
        let style = if index == tui.artist_tab {
            p.subtle_selected()
        } else {
            p.hint()
        };
        frame.render_widget(Paragraph::new(format!(" {label} ")).style(style), rect);
        tui.areas.hits.push((
            rect,
            Hit::Artist {
                tab: index,
                index: tui.artist_selected[index],
            },
        ));
        x += width + 1;
    }

    let body = Rect {
        y: tab_y + 1,
        height: area.height.saturating_sub(header.height + 1),
        ..area
    };
    if body.height == 0 {
        return;
    }
    tui.artist_rows = body.height as usize;
    tui.areas.content = body;
    let len = artist_count(view, tui.artist_tab);
    tui.artist_selected[tui.artist_tab] =
        tui.artist_selected[tui.artist_tab].min(len.saturating_sub(1));
    ensure_visible(
        tui.artist_selected[tui.artist_tab],
        &mut tui.artist_offset[tui.artist_tab],
        body.height as usize,
    );
    tui.areas.content_offset = tui.artist_offset[tui.artist_tab];

    if len == 0 {
        draw_empty(tui, view, &ContentState::Empty, p, body, frame);
        return;
    }
    let offset = tui.artist_offset[tui.artist_tab];
    for (row_index, index) in (offset..len).take(body.height as usize).enumerate() {
        let selected = index == tui.artist_selected[tui.artist_tab];
        let text = match tui.artist_tab {
            0 => artist
                .detail
                .hot_songs
                .get(index)
                .map(|song| {
                    song_line(
                        index,
                        &song.name,
                        &song.singer,
                        &song.album,
                        song.duration,
                        false,
                        area.width as usize,
                    )
                })
                .unwrap_or_default(),
            1 => artist
                .albums
                .get(index)
                .map(|album| {
                    format!(
                        " {:<3} {}  {} 首",
                        index + 1,
                        fit(&album.name, area.width.saturating_sub(14) as usize),
                        album.size
                    )
                })
                .unwrap_or_default(),
            _ => artist
                .similar
                .get(index)
                .map(|singer| format!(" {:<3} {}", index + 1, singer.name))
                .unwrap_or_default(),
        };
        let rect = Rect {
            y: body.y + row_index as u16,
            height: 1,
            ..body
        };
        frame.render_widget(
            Paragraph::new(fit(&text, area.width as usize)).style(if selected {
                p.selected()
            } else {
                Style::default().fg(p.text)
            }),
            rect,
        );
        tui.areas.hits.push((
            rect,
            Hit::Artist {
                tab: tui.artist_tab,
                index,
            },
        ));
    }
}

fn draw_queue(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    if area.height == 0 {
        return;
    }
    let mut keys = Vec::new();
    for key in &view.player.queue_keys {
        keys.push(Span::styled(
            format!(" {} ", fit(key, 18)),
            if *key == view.player.queue_key {
                p.subtle_selected()
            } else {
                p.hint()
            },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(keys)), Rect { height: 1, ..area });
    let body = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    if body.height == 0 {
        return;
    }
    tui.queue_rows = body.height as usize;
    tui.areas.content = body;
    let len = view.player.queue.len();
    tui.queue_selected = tui.queue_selected.min(len.saturating_sub(1));
    ensure_visible(
        tui.queue_selected,
        &mut tui.queue_offset,
        body.height as usize,
    );
    tui.areas.content_offset = tui.queue_offset;

    if len == 0 {
        frame.render_widget(
            Paragraph::new("  队列为空 · 从任意歌曲列表按 Enter 开始播放")
                .style(Style::default().fg(p.muted))
                .alignment(Alignment::Center),
            body,
        );
        return;
    }
    let offset = tui.queue_offset;
    for (row_index, index) in (offset..len).take(body.height as usize).enumerate() {
        let song = &view.player.queue[index];
        let current = view.player.queue_index == Some(index);
        let selected = index == tui.queue_selected;
        let text = song_line(
            index,
            &song.name,
            &song.singer,
            &song.album,
            song.duration,
            current,
            area.width as usize,
        );
        let rect = Rect {
            y: body.y + row_index as u16,
            height: 1,
            ..body
        };
        let style = if selected {
            p.selected()
        } else if current {
            Style::default().fg(p.accent)
        } else {
            Style::default().fg(p.text)
        };
        frame.render_widget(Paragraph::new(text).style(style), rect);
        tui.areas.hits.push((rect, Hit::Queue(index)));
    }
}

fn draw_lyrics(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    if view.player.lyrics.is_empty() {
        frame.render_widget(
            Paragraph::new("  暂无歌词")
                .style(Style::default().fg(p.muted))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let current = current_lyric_index(&view.player.lyrics, view.player.position_secs);
    let style = view.settings.lyric_style;
    let current_style = match style {
        crate::config::LyricStyle::Plain => Style::default().fg(p.text),
        crate::config::LyricStyle::Ktv => Style::default()
            .fg(p.accent)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        crate::config::LyricStyle::Flow => Style::default()
            .fg(p.accent)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        crate::config::LyricStyle::Window | crate::config::LyricStyle::OneLine => {
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
        }
    };
    let lyric_time = art_time(tui);
    let shimmer = art::Shimmer {
        period: 3.4,
        width: 6.0,
        base: p.accent,
        highlight: p.text,
    };
    let mut lines: Vec<Line> = Vec::new();
    let mut current_line = 0usize;
    for (index, lyric) in view.player.lyrics.iter().enumerate() {
        if Some(index) == current {
            current_line = lines.len();
        }
        let line_style = if Some(index) == current {
            current_style
        } else if style == crate::config::LyricStyle::Plain {
            Style::default().fg(p.text)
        } else if current.is_some_and(|current| index < current) {
            Style::default().fg(p.muted)
        } else {
            Style::default().fg(p.text)
        };
        let text = format!("  {} ", lyric.text);
        let text_len = text.chars().count();
        if Some(index) == current && style != crate::config::LyricStyle::Plain {
            let spans = text
                .chars()
                .enumerate()
                .map(|(position, ch)| {
                    Span::styled(
                        ch.to_string(),
                        line_style.fg(shimmer.color_at(position, text_len, lyric_time)),
                    )
                })
                .collect::<Vec<_>>();
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(Span::styled(text, line_style)));
        }
        if let Some(translated) = translation_at(&view.player.translated_lyrics, lyric)
            && view.settings.lyric_translation
        {
            lines.push(Line::from(Span::styled(
                format!("  {} {translated}", tui.symbols.translation),
                if Some(index) == current {
                    Style::default().fg(p.warning)
                } else {
                    Style::default().fg(p.muted)
                },
            )));
        }
    }
    let visible = area.height as usize;
    if style == crate::config::LyricStyle::OneLine {
        let start = current_line;
        let count = if view.settings.lyric_translation
            && current
                .and_then(|index| view.player.lyrics.get(index))
                .and_then(|lyric| translation_at(&view.player.translated_lyrics, lyric))
                .is_some()
        {
            2
        } else {
            1
        };
        let picked = lines
            .into_iter()
            .skip(start)
            .take(count)
            .collect::<Vec<_>>();
        let mut centred = vec![Line::default(); visible.saturating_sub(picked.len()) / 2];
        centred.extend(picked);
        frame.render_widget(
            Paragraph::new(centred)
                .style(p.panel())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let start = current_line.saturating_sub(visible / 2);
    let lyrics = lines
        .into_iter()
        .skip(start)
        .take(visible)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lyrics)
            .style(p.panel())
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_login(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let mut x = area.x;
    for (index, method) in LoginMethod::ALL.iter().enumerate() {
        let label = format!(" {} ", method.label());
        let width = UnicodeWidthStr::width(label.as_str()) as u16;
        let rect = Rect {
            x,
            y: area.y,
            width,
            height: 1,
        };
        if rect.right() > area.right() {
            break;
        }
        frame.render_widget(
            Paragraph::new(label).style(if *method == view.login.method {
                p.subtle_selected()
            } else {
                p.hint()
            }),
            rect,
        );
        tui.areas.hits.push((rect, Hit::LoginMethod(index)));
        x += width + 1;
    }

    let body = Rect {
        y: area.y + 2,
        height: area.height.saturating_sub(2),
        ..area
    };
    if body.height == 0 {
        return;
    }
    match view.login.method {
        LoginMethod::Qr => {
            let footer = u16::from(body.height > 2);
            if let Some(qr) = &tui.qr {
                let width = qr
                    .iter()
                    .map(|line| UnicodeWidthStr::width(line.as_str()))
                    .max()
                    .unwrap_or(0) as u16;
                let height = qr.len() as u16;
                // Two rows below the code hold the status and the key hint. If
                // the window cannot show the whole code, do not paint a
                // half-QR that a phone cannot scan.
                if height + footer < body.height {
                    let qr_area = Rect {
                        x: body.x + body.width.saturating_sub(width) / 2,
                        y: body.y,
                        width: width.min(body.width),
                        height,
                    };
                    let lines = qr
                        .iter()
                        .map(|line| {
                            Line::from(Span::styled(line.clone(), Style::default().fg(p.text)))
                        })
                        .collect::<Vec<_>>();
                    frame.render_widget(Paragraph::new(lines), qr_area);
                    let status_y = qr_area.y + qr_area.height;
                    frame.render_widget(
                        Paragraph::new(format!("  {}", view.login.qr_status))
                            .style(Style::default().fg(p.muted)),
                        Rect {
                            y: status_y,
                            height: 1,
                            ..body
                        },
                    );
                } else {
                    frame.render_widget(
                        Paragraph::new("  窗口太矮，二维码显示不下\n  Ctrl+2 用密码登录")
                            .style(p.hint())
                            .alignment(Alignment::Center),
                        Rect {
                            height: body.height.saturating_sub(footer),
                            ..body
                        },
                    );
                }
            } else {
                frame.render_widget(
                    Paragraph::new("  正在生成二维码…")
                        .style(Style::default().fg(p.muted))
                        .alignment(Alignment::Center),
                    Rect {
                        height: body.height.saturating_sub(footer),
                        ..body
                    },
                );
            }
            if footer > 0 {
                frame.render_widget(
                    Paragraph::new("  Enter/r 刷新二维码 · Esc 返回")
                        .style(p.hint())
                        .alignment(Alignment::Center),
                    Rect {
                        y: body.bottom() - 1,
                        height: 1,
                        ..body
                    },
                );
            }
        }
        LoginMethod::Sign => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled("  每日签到", p.title())),
                    Line::from(Span::styled("  Enter 签到", p.hint())),
                ]),
                body,
            );
        }
        LoginMethod::Password | LoginMethod::Sms => {
            let fields = login_fields(view);
            let mut y = body.y;
            for (index, field) in fields.iter().enumerate() {
                if y >= body.bottom() {
                    break;
                }
                let label = match field {
                    LoginField::Account => "账号",
                    LoginField::Password => "密码",
                    LoginField::Phone => "手机",
                    LoginField::Code => "验证码",
                };
                let value = field_value(view, *field);
                let shown = if field.masked() {
                    "*".repeat(value.chars().count())
                } else {
                    value.to_string()
                };
                let selected = tui.focus == Focus::Content && index == tui.login_field;
                let line = Line::from(vec![
                    Span::styled(format!("  {} ", fit(label, 6)), p.hint()),
                    Span::styled(
                        fit(&shown, 32),
                        if selected {
                            p.selected()
                        } else {
                            Style::default().fg(p.text)
                        },
                    ),
                ]);
                frame.render_widget(
                    Paragraph::new(line),
                    Rect {
                        y,
                        height: 1,
                        ..body
                    },
                );
                y += 2;
            }
            if y < body.bottom() {
                let hint = match view.login.method {
                    LoginMethod::Sms => "  Tab 切换字段 · Enter 登录 · Ctrl+S 发送验证码",
                    _ => "  Tab 切换字段 · Enter 登录",
                };
                frame.render_widget(
                    Paragraph::new(hint).style(p.hint()),
                    Rect {
                        y,
                        height: 1,
                        ..body
                    },
                );
            }
        }
    }

    if let Some(error) = &view.login.error {
        frame.render_widget(
            Paragraph::new(format!("  {error}")).style(Style::default().fg(p.error)),
            Rect {
                y: body.bottom().saturating_sub(1),
                height: 1,
                ..body
            },
        );
    } else if let Some(notice) = &view.login.notice {
        frame.render_widget(
            Paragraph::new(format!("  {notice}")).style(Style::default().fg(p.accent)),
            Rect {
                y: body.bottom().saturating_sub(1),
                height: 1,
                ..body
            },
        );
    }
}

fn draw_settings(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let values = [
        view.settings.theme.clone(),
        view.settings.quality.clone(),
        on_off(view.settings.save_on_play),
        on_off(view.settings.notify_song_change),
        on_off(view.settings.notify_errors),
        on_off(view.settings.lyric_translation),
        on_off(view.settings.visualizer),
        on_off(view.settings.pitch),
        if view.user.is_some() {
            "签到".into()
        } else {
            "需登录".into()
        },
        if view.user.is_some() {
            "退出".into()
        } else {
            "去登录".into()
        },
    ];
    tui.areas.content = area;
    tui.areas.content_offset = 0;
    for (index, label) in SETTINGS.iter().enumerate() {
        if index as u16 >= area.height {
            break;
        }
        let selected = index == tui.settings_selected;
        let text = format!(
            "  {} {}",
            fit(label, 12),
            fit(&values[index], area.width.saturating_sub(16) as usize)
        );
        let rect = Rect {
            y: area.y + index as u16,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(fit(&text, area.width as usize)).style(if selected {
                p.selected()
            } else {
                Style::default().fg(p.text)
            }),
            rect,
        );
        tui.areas.hits.push((rect, Hit::Settings(index)));
    }
}

fn draw_player(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let time = art_time(tui);
    let playing = view.player.playing && !view.player.paused;
    art::Aurora::new(time, art_palette(p))
        .intensity(if playing { 0.34 } else { 0.14 })
        .render(area, frame.buffer_mut());
    let border = if playing {
        art::Pulse {
            period: 2.6,
            min: 0.0,
            max: 1.0,
        }
        .color(time, p.border, p.accent)
    } else {
        p.border
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let cover_width = if inner.width >= 60 { 9 } else { 0 };
    let (cover_area, info_area) = if cover_width > 0 {
        let columns =
            Layout::horizontal([Constraint::Length(cover_width), Constraint::Min(10)]).split(inner);
        (Some(columns[0]), columns[1])
    } else {
        (None, inner)
    };
    if let Some(cover_area) = cover_area {
        if tui.cover.is_some() {
            tui.draw_cover(cover_area, frame);
        } else {
            frame.render_widget(
                Paragraph::new(" ♪ ")
                    .style(Style::default().fg(p.accent).add_modifier(Modifier::BOLD))
                    .alignment(Alignment::Center),
                cover_area,
            );
        }
    }

    let song = view.player.song.as_ref();
    let title = song.map(|song| song.name.as_str()).unwrap_or("未播放");
    let artist = song.map(|song| song.singer.as_str()).unwrap_or("");
    let album = song.map(|song| song.album.as_str()).unwrap_or("");
    let state = if view.player.playing && !view.player.paused {
        "播放中"
    } else if view.player.paused {
        "已暂停"
    } else {
        "已停止"
    };
    let mode_name = match view.player.mode {
        crate::playback::PlayMode::Sequential => "顺序",
        crate::playback::PlayMode::RepeatOne => "单曲",
        crate::playback::PlayMode::RepeatAll => "列表",
        crate::playback::PlayMode::Shuffle => "随机",
        crate::playback::PlayMode::Heartbeat { .. } => "心动",
    };
    let like = if view.player.liked { "♥" } else { "♡" };
    let cache = if view.player.cached { " ⛁" } else { "" };
    let title_line = Line::from(vec![
        Span::styled(format!(" {title} "), p.title()),
        Span::styled(
            format!("{state}{cache}  {like}"),
            if view.player.liked {
                Style::default().fg(p.accent)
            } else {
                p.hint()
            },
        ),
    ]);
    let detail_line = Line::from(vec![
        Span::styled(format!(" {artist}"), Style::default().fg(p.text)),
        Span::styled(format!("  ·  {album}"), p.hint()),
        Span::styled(format!("   {mode_name}"), p.hint()),
    ]);
    frame.render_widget(
        Paragraph::new(vec![title_line, detail_line]),
        Rect {
            height: 2.min(info_area.height),
            ..info_area
        },
    );

    if info_area.width >= 14 {
        let icon = if view.player.volume < 0.34 {
            &tui.symbols.volume_low
        } else if view.player.volume < 0.67 {
            &tui.symbols.volume_mid
        } else {
            &tui.symbols.volume_high
        };
        let volume = format!("{icon} 音量 {:>3.0}%", view.player.volume * 100.0);
        let volume_area = Rect {
            x: info_area.right().saturating_sub(13),
            y: info_area.y + 1,
            width: 13,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(volume).style(Style::default().fg(p.muted)),
            volume_area,
        );
        tui.areas.hits.push((volume_area, Hit::Volume));
    }

    if info_area.height >= 3 {
        let gauge_area = Rect {
            y: info_area.y + 2,
            height: 1,
            ..info_area
        };
        let label = format!(
            "{} / {}",
            format_duration((view.player.position_secs * 1000.0) as u64),
            format_duration((view.player.duration_secs * 1000.0) as u64)
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(p.accent).bg(p.surface))
            .ratio(view.player.progress.clamp(0.0, 1.0))
            .label(label);
        frame.render_widget(gauge, gauge_area);
        tui.areas.hits.push((gauge_area, Hit::Gauge));
    }

    if info_area.height >= 4 {
        let controls_area = Rect {
            y: info_area.y + 3,
            height: 1,
            ..info_area
        };
        let controls = "  [⏮]  [⏯]  [⏭]   n/p 切歌  Space 播放    -/= 音量";
        frame.render_widget(
            Paragraph::new(controls).style(Style::default().fg(p.text)),
            controls_area,
        );
        let mut x = controls_area.x + UnicodeWidthStr::width("  ") as u16;
        for (svg, hit) in [("[⏮]", Hit::Prev), ("[⏯]", Hit::Play), ("[⏭]", Hit::Next)] {
            let width = UnicodeWidthStr::width(svg) as u16;
            tui.areas.hits.push((
                Rect {
                    x,
                    width,
                    height: 1,
                    ..controls_area
                },
                hit,
            ));
            x = x.saturating_add(width + UnicodeWidthStr::width("  ") as u16);
        }
    }

    if view.settings.visualizer && !view.player.visualizer.is_empty() && info_area.height >= 5 {
        let width = info_area.width.saturating_sub(2) as usize;
        let bars = view
            .player
            .visualizer
            .iter()
            .take(width)
            .map(|value| {
                let bars: Vec<char> = tui.symbols.visualizer_bars.chars().collect();
                let index = ((*value).clamp(0.0, 1.0) * (bars.len() - 1) as f32).round() as usize;
                bars[index]
            })
            .collect::<String>();
        frame.render_widget(
            Paragraph::new(format!(" {bars}")).style(Style::default().fg(p.accent)),
            Rect {
                y: info_area.bottom().saturating_sub(1),
                height: 1,
                ..info_area
            },
        );
    }

    if let Some(pitch) = &view.player.pitch {
        frame.render_widget(
            Paragraph::new(format!(" {pitch}")).style(Style::default().fg(p.warning)),
            Rect {
                x: info_area.right().saturating_sub(10),
                width: 10,
                y: info_area.y,
                height: 1,
            },
        );
    }
}

fn draw_footer(
    tui: &mut Tui,
    view: &View,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    if let Some(command) = &tui.command {
        let line = Line::from(vec![
            Span::styled(":", p.title()),
            Span::styled(command.text.clone(), Style::default().fg(p.text)),
        ]);
        frame.render_widget(Paragraph::new(line).style(p.panel()), area);
        if area.width > 1 {
            let before = &command.text[..char_boundary(&command.text, command.cursor)];
            let x = area.x + 1 + UnicodeWidthStr::width(before) as u16;
            frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
        }
        return;
    }
    if let Some(notice) = tui.visible_notice() {
        frame.render_widget(
            Paragraph::new(format!(" {notice}")).style(Style::default().fg(p.warning)),
            area,
        );
        return;
    }
    let hints = match view.page {
        Page::Login => " Ctrl+1..4 / Tab 方法 · Enter 登录 · Esc 返回 · ? 帮助",
        Page::Settings => " ↑↓ 选择 · Enter 切换 · Tab 焦点 · Esc 返回 · ? 帮助",
        Page::Queue => " ↑↓ 选择 · Enter 播放 · [ ] 切换队列 · w 清空 · Tab 焦点",
        Page::Lyrics => " Space 播放 · n/p 切歌 · y 译文 · f 队列 · , 设置",
        Page::Artist => " 1/2/3 分区 · ↑↓ 选择 · Enter 打开 · s 收藏 · a 下一首",
        Page::Browse => " ↑↓ 选择 · Enter 播放/打开 · / 搜索 · f 队列 · l 歌词 · ? 帮助",
    };
    frame.render_widget(
        Paragraph::new(fit(hints, area.width as usize)).style(p.hint()),
        area,
    );
}

fn draw_help(p: &super::theme::Palette, area: Rect, frame: &mut Frame) {
    let width = area.width.min(78);
    let height = area.height.min(22);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(
        Fill::new(" ").style(Style::default().bg(p.background)),
        popup,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(p.accent))
        .title(Span::styled(" 快捷键 ", p.title()))
        .style(p.panel());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let lines = [
        ("Tab / Shift+Tab", "切换 导航 / 内容 / 搜索"),
        ("↑ ↓ / j k", "移动选择；gg / G 由 Home / End 承担"),
        ("Enter", "播放、打开详情或执行设置项"),
        ("Space", "播放 / 暂停"),
        ("n / p", "下一首 / 上一首"),
        ("m / S", "播放模式 / 收藏当前歌曲"),
        ("a / s", "加入下一首 / 收藏光标行"),
        ("A / b", "打开歌手 / 专辑"),
        ("o", "加载更多"),
        ("c / C", "包含这首歌的歌单 / 相似歌曲"),
        ("/", "搜索；Tab 切换类型；Enter 提交"),
        (":", "命令模式（:help、:theme、:volume 70、:seek +15）"),
        ("f / l / , / L / g", "队列 / 歌词 / 设置 / 登录 / 浏览"),
        ("- / =", "音量减 / 加"),
        ("[ / ]", "后退 / 前进；队列页切换队列"),
        ("r", "刷新当前列表"),
        ("q / Ctrl+C", "退出"),
    ];
    let content = lines
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled(format!("  {key:<18}"), p.title()),
                Span::styled(*description, Style::default().fg(p.text)),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(content), inner);
}

fn page_heading(view: &View) -> String {
    match view.page {
        Page::Browse | Page::Artist => view.title.clone(),
        Page::Queue => format!("播放队列 · {} 首", view.player.queue.len()),
        Page::Lyrics => match &view.player.song {
            Some(song) => format!("歌词 · {}", song.name),
            None => "歌词".into(),
        },
        Page::Login => "登录 / 账号".into(),
        Page::Settings => "设置".into(),
    }
}

fn song_header(width: u16) -> String {
    let (title, artist, album) = song_widths(width as usize);
    format!(
        "  #  {} {} {} 时长",
        fit("标题", title),
        fit("歌手", artist),
        fit("专辑", album)
    )
}

fn song_line(
    index: usize,
    name: &str,
    singer: &str,
    album: &str,
    duration_ms: u64,
    current: bool,
    width: usize,
) -> String {
    let (title, artist, album_width) = song_widths(width);
    format!(
        "{} {:>3} {} {} {} {}",
        if current { "▶" } else { " " },
        index + 1,
        fit(name, title),
        fit(singer, artist),
        fit(album, album_width),
        format_duration(duration_ms)
    )
}

fn song_widths(width: usize) -> (usize, usize, usize) {
    let duration = 6;
    let index = 5;
    let available = width.saturating_sub(index + duration + 3);
    let title = (available * 42 / 100).max(10);
    let artist = (available * 25 / 100).max(8);
    let album = available.saturating_sub(title + artist).max(8);
    (title, artist, album)
}

fn list_header(width: u16) -> String {
    let name = width.saturating_sub(22) as usize;
    format!("  #  {}  创建者", fit("名称", name.max(8)))
}

fn draw_empty(
    tui: &mut Tui,
    _view: &View,
    content: &ContentState,
    p: &super::theme::Palette,
    area: Rect,
    frame: &mut Frame,
) {
    let text = match content {
        ContentState::Loading => {
            format!(
                "{} 正在加载…",
                tui.symbols.spinner_activity[(tui.tick as usize
                    / tui.symbols.spinner_ticks_per_frame.max(1) as usize)
                    % tui.symbols.spinner_activity.len()]
            )
        }
        ContentState::Error(error) => format!("{error}\n\nr 重试"),
        _ => "这里还没有内容".to_string(),
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(
                Style::default().fg(if matches!(content, ContentState::Error(_)) {
                    p.error
                } else {
                    p.muted
                }),
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn on_off(value: bool) -> String {
    if value { "开".into() } else { "关".into() }
}

fn fit(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(width);
    let mut used = 0;
    for ch in text.chars() {
        let cell = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cell > width {
            break;
        }
        out.push(ch);
        used += cell;
    }
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

fn truncate(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let cell = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cell > width {
            break;
        }
        out.push(ch);
        used += cell;
    }
    out
}

fn char_boundary(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}
