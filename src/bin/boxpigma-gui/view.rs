//! Native iced rendering for backend snapshots.
//!
//! Every click becomes a semantic [`backend::Message`]; nothing here talks to
//! the network or the audio engine.

use std::sync::Arc;

use boxpigma::{
    backend::{self, ContentSource, Page, SearchKind, View, current_lyric_index, translation_at},
    playback::mode_icon,
    state::{LoginField, LoginMethod},
    utils::format_duration,
};
use iced::{
    Alignment, Border, Color, ContentFit, Element, Length, Theme,
    widget::{
        Column, Row, button, column, container, image, qr_code, row, scrollable, slider, space,
        text, text_input,
    },
};

use crate::app::{Gui, Local, UiMessage, theme as ui_theme};

impl Gui {
    pub fn render<'a>(&'a self, view: &'a View) -> Element<'a, UiMessage> {
        let theme = ui_theme(view);
        let body = column![
            self.top_bar(view, &theme),
            self.page(view, &theme),
            self.player_bar(view, &theme),
        ]
        .spacing(8)
        .height(Length::Fill);

        container(
            row![self.sidebar(view, &theme), body]
                .spacing(8)
                .height(Length::Fill),
        )
        .padding(10)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.palette().background.into()),
            ..container::Style::default()
        })
        .into()
    }

    fn sidebar<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let mut items = Column::new().spacing(2);
        for (section_index, section) in view.nav.sections.iter().enumerate() {
            items = items.push(
                text(section.title.clone())
                    .size(11)
                    .color(theme.palette().primary)
                    .width(Length::Fill),
            );
            for (item_index, item) in section.items.iter().enumerate() {
                let selected = view.nav.section == section_index && view.nav.item == item_index;
                items = items.push(
                    button(text(item.name.clone()).size(14))
                        .width(Length::Fill)
                        .padding([5, 8])
                        .style(if selected {
                            button::primary
                        } else {
                            button::text
                        })
                        .on_press(UiMessage::User(backend::Message::ActivateNav {
                            section: section_index,
                            item: item_index,
                        })),
                );
            }
            items = items.push(space().height(Length::Fixed(6.0)));
        }

        items = items.push(space().height(Length::Fixed(8.0)));
        for (page, label) in [
            (Page::Queue, "播放队列"),
            (Page::Lyrics, "歌词"),
            (Page::Login, "登录 / 账号"),
            (Page::Settings, "设置"),
        ] {
            let selected = view.page == page;
            items = items.push(
                button(text(label).size(14))
                    .width(Length::Fill)
                    .padding([5, 8])
                    .style(if selected {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(UiMessage::User(backend::Message::Navigate(page))),
            );
        }

        let session = match &view.user {
            Some(user) => format!("已登录 · {}", user.nickname),
            None => "未登录".into(),
        };
        items = items.push(space().height(Length::Fixed(10.0)));
        items = items.push(text(session).size(11).color(muted(theme)));
        items = items.push(
            text("Ctrl+空格 播放/暂停\nCtrl+←/→ 上一首/下一首")
                .size(10)
                .color(muted(theme)),
        );

        container(scrollable(items).height(Length::Fill))
            .padding(10)
            .width(Length::Fixed(210.0))
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.weak.color.into()),
                border: Border {
                    radius: 10.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    fn top_bar<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let heading = match view.page {
            Page::Browse | Page::Artist => view.title.clone(),
            Page::Queue => format!("播放队列 · {} 首", view.player.queue.len()),
            Page::Lyrics => match &view.player.song {
                Some(song) => format!("歌词 · {}", song.name),
                None => "歌词".into(),
            },
            Page::Login => "登录 / 账号".into(),
            Page::Settings => "设置".into(),
        };
        let mut title_row = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(text(heading).size(22));
        if let Some(subtitle) = &view.subtitle {
            title_row = title_row.push(text(subtitle.clone()).size(12).color(muted(theme)));
        }
        if view.loading {
            title_row = title_row.push(text("加载中…").size(11).color(theme.palette().warning));
        }
        title_row = title_row.push(space().width(Length::Fill));
        if view.can_back {
            title_row = title_row.push(
                button("返回")
                    .padding([3, 10])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::Back)),
            );
        }
        title_row = title_row.push(
            button("刷新")
                .padding([3, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::Refresh)),
        );

        let mut kinds = Row::new().spacing(4);
        for kind in SearchKind::ALL {
            let selected = view.search.kind == kind;
            kinds = kinds.push(
                button(text(kind.label()).size(11))
                    .padding([3, 8])
                    .style(if selected {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(UiMessage::User(backend::Message::SetSearchKind(kind))),
            );
        }

        let search = row![
            text_input(
                "搜索歌曲 / 歌单 / 歌手 / 专辑，回车确认",
                &view.search.query
            )
            .on_input(|value| UiMessage::User(backend::Message::SearchInput(value)))
            .on_submit(UiMessage::User(backend::Message::SearchSubmit))
            .padding(7)
            .width(Length::Fill),
            kinds,
            button("搜索")
                .padding([5, 12])
                .style(button::primary)
                .on_press(UiMessage::User(backend::Message::SearchSubmit)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let banner = text(view.banner.text.clone())
            .size(11)
            .color(if view.banner.error {
                theme.palette().danger
            } else {
                muted(theme)
            });

        column![title_row, search, banner]
            .spacing(6)
            .width(Length::Fill)
            .into()
    }

    fn page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        match view.page {
            Page::Browse => self.browse_page(view, theme),
            Page::Artist => self.artist_page(view, theme),
            Page::Queue => self.queue_page(view, theme),
            Page::Lyrics => self.lyrics_page(view, theme),
            Page::Login => self.login_page(view, theme),
            Page::Settings => self.settings_page(view, theme),
        }
    }

    /* ------------------------------------------------------------------ */
    /*                              browse                                */
    /* ------------------------------------------------------------------ */

    fn browse_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let content: Element<'_, UiMessage> = match &view.content {
            boxpigma::state::ContentState::Empty => empty(theme, "这一页还没有内容"),
            boxpigma::state::ContentState::Loading => empty(theme, "加载中…"),
            boxpigma::state::ContentState::Error(error) => column![
                text(error.clone()).size(13).color(theme.palette().danger),
                button("重试").on_press(UiMessage::User(backend::Message::Refresh)),
            ]
            .spacing(8)
            .into(),
            boxpigma::state::ContentState::Songs(songs) => {
                let mut rows = Column::new().spacing(2);
                for (index, song) in songs.iter().enumerate() {
                    rows = rows.push(self.content_song_row(view, index, song, theme));
                }
                scrollable(rows).height(Length::Fill).into()
            }
            boxpigma::state::ContentState::SongLists(lists) => {
                let mut rows = Column::new().spacing(2);
                for (index, list) in lists.iter().enumerate() {
                    let action = if list.author.is_empty() {
                        list.name.clone()
                    } else {
                        format!("{}  ·  {}", list.name, list.author)
                    };
                    rows = rows.push(
                        button(text(action).size(14))
                            .width(Length::Fill)
                            .padding([7, 10])
                            .style(button::text)
                            .on_press(UiMessage::User(backend::Message::ActivateRow(index))),
                    );
                }
                scrollable(rows).height(Length::Fill).into()
            }
            boxpigma::state::ContentState::TopLists(lists) => {
                let mut rows = Column::new().spacing(2);
                for (index, list) in lists.iter().enumerate() {
                    let label = if list.update.is_empty() {
                        list.name.clone()
                    } else {
                        format!("{}  ·  {}", list.name, list.update)
                    };
                    rows = rows.push(
                        button(column![
                            text(label).size(14),
                            text(list.description.clone()).size(11).color(muted(theme)),
                        ])
                        .width(Length::Fill)
                        .padding([7, 10])
                        .style(button::text)
                        .on_press(UiMessage::User(backend::Message::ActivateRow(index))),
                    );
                }
                scrollable(rows).height(Length::Fill).into()
            }
            boxpigma::state::ContentState::Singers(singers) => {
                let mut rows = Column::new().spacing(2);
                for (index, singer) in singers.iter().enumerate() {
                    rows = rows.push(
                        button(text(singer.name.clone()).size(14))
                            .width(Length::Fill)
                            .padding([7, 10])
                            .style(button::text)
                            .on_press(UiMessage::User(backend::Message::ActivateRow(index))),
                    );
                }
                scrollable(rows).height(Length::Fill).into()
            }
            boxpigma::state::ContentState::HotSearch(keywords) => {
                let mut rows = Column::new().spacing(6);
                for chunk in keywords.0.chunks(3) {
                    let mut line = Row::new().spacing(6);
                    for keyword in chunk {
                        line = line.push(
                            button(text(keyword.clone()).size(13))
                                .padding([6, 10])
                                .on_press(UiMessage::User(backend::Message::SearchInput(
                                    keyword.clone(),
                                ))),
                        );
                    }
                    rows = rows.push(line);
                }
                scrollable(rows).height(Length::Fill).into()
            }
        };

        let mut body = Column::new().spacing(8).height(Length::Fill);
        if let Some(pagination) = &view.pagination {
            let label = format!("已加载 {}/{} 首", view.content.len(), pagination.total);
            let mut footer = Row::new().spacing(8).align_y(Alignment::Center);
            footer = footer.push(text(label).size(11).color(muted(theme)));
            if pagination.has_more {
                footer = footer.push(
                    button(if pagination.loading {
                        "加载中…"
                    } else {
                        "加载更多"
                    })
                    .padding([3, 10])
                    .on_press(UiMessage::User(backend::Message::LoadMore)),
                );
            }
            body = body.push(footer);
        }
        body = body.push(content);
        body.into()
    }

    fn content_song_row<'a>(
        &'a self,
        view: &'a View,
        index: usize,
        song: &'a Arc<ncm_api::SongInfo>,
        theme: &Theme,
    ) -> Element<'a, UiMessage> {
        let playing = view
            .player
            .song
            .as_ref()
            .is_some_and(|current| current.id == song.id);
        let cached = view.player.cached && playing;
        let local_source = matches!(
            &view.source,
            Some(ContentSource::Endpoint(api)) if api == "download" || api == "local_music"
        );
        let title = format!(
            "{} {:>3}. {}  ·  {}",
            if playing { "▶" } else { " " },
            index + 1,
            song.name,
            song.singer
        );
        let main = button(
            row![
                text(title).size(13).width(Length::Fill),
                text(if cached { "已缓存" } else { "" })
                    .size(10)
                    .color(muted(theme)),
                text(format_duration(song.duration))
                    .size(11)
                    .color(muted(theme)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([5, 8])
        .style(if playing {
            button::primary
        } else {
            button::text
        })
        .on_press(UiMessage::User(backend::Message::ActivateRow(index)));

        let mut actions = Row::new().spacing(2);
        actions = actions.push(
            button("＋")
                .padding([3, 7])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::AddNextRow(index))),
        );
        actions = actions.push(
            button("♥")
                .padding([3, 7])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::LikeRow(index))),
        );
        if song.artist_id != 0 {
            actions = actions.push(
                button("歌手")
                    .padding([3, 7])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::OpenArtistRow(index))),
            );
        }
        if song.album_id != 0 {
            actions = actions.push(
                button("专辑")
                    .padding([3, 7])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::OpenAlbumRow(index))),
            );
        }
        if song.local_path.is_some() || cached || local_source {
            actions = actions.push(
                button("云")
                    .padding([3, 7])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::UploadRow(index))),
            );
        }
        row![main, actions]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
    }

    /* ------------------------------------------------------------------ */
    /*                              artist                                */
    /* ------------------------------------------------------------------ */

    fn artist_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let Some(artist) = &view.artist else {
            return empty(
                theme,
                if view.loading {
                    "正在加载歌手…"
                } else {
                    "没有歌手信息"
                },
            );
        };
        let mut body = Column::new().spacing(8);
        let alias = if artist.detail.alias.is_empty() {
            String::new()
        } else {
            format!("  ·  {}", artist.detail.alias.join(" / "))
        };
        body = body.push(
            column![
                text(format!("{}{alias}", artist.detail.name)).size(22),
                text(format!(
                    "{} 首歌曲  ·  {} 张专辑",
                    artist.detail.music_size, artist.detail.album_size
                ))
                .size(11)
                .color(muted(theme)),
                text(artist.detail.brief_desc.clone())
                    .size(12)
                    .color(muted(theme)),
            ]
            .spacing(3),
        );

        if !artist.detail.hot_songs.is_empty() {
            body = body.push(text("热门歌曲").size(15).color(theme.palette().primary));
            let mut songs = Column::new().spacing(2);
            for (index, song) in artist.detail.hot_songs.iter().enumerate() {
                let playing = view
                    .player
                    .song
                    .as_ref()
                    .is_some_and(|current| current.id == song.id);
                let title = format!(
                    "{} {:>2}. {}  ·  {}",
                    if playing { "▶" } else { " " },
                    index + 1,
                    song.name,
                    song.singer
                );
                let main = button(text(title).size(13))
                    .width(Length::Fill)
                    .padding([5, 8])
                    .style(if playing {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(UiMessage::User(backend::Message::PlayArtistSong(index)));
                let actions = row![
                    button("＋")
                        .padding([3, 7])
                        .style(button::text)
                        .on_press(UiMessage::User(backend::Message::AddNextArtistSong(index))),
                    button("♥")
                        .padding([3, 7])
                        .style(button::text)
                        .on_press(UiMessage::User(backend::Message::LikeArtistSong(index))),
                    text(format_duration(song.duration))
                        .size(11)
                        .color(muted(theme)),
                ]
                .spacing(3)
                .align_y(Alignment::Center);
                songs = songs.push(row![main, actions].spacing(4).align_y(Alignment::Center));
            }
            body = body.push(songs);
        }

        if !artist.albums.is_empty() {
            body = body.push(text("专辑").size(15).color(theme.palette().primary));
            let mut albums = Column::new().spacing(2);
            for (index, album) in artist.albums.iter().enumerate() {
                albums = albums.push(
                    button(
                        row![
                            text(album.name.clone()).size(13).width(Length::Fill),
                            text(format!("{} 首", album.size))
                                .size(11)
                                .color(muted(theme)),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([5, 8])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::OpenArtistAlbum(index))),
                );
            }
            body = body.push(albums);
        }

        if !artist.similar.is_empty() {
            body = body.push(text("相似歌手").size(15).color(theme.palette().primary));
            let mut line = Row::new().spacing(6);
            for (index, singer) in artist.similar.iter().enumerate() {
                line = line.push(
                    button(text(singer.name.clone()).size(12))
                        .padding([4, 8])
                        .style(button::text)
                        .on_press(UiMessage::User(backend::Message::OpenSimilarArtist(index))),
                );
            }
            body = body.push(line);
        }

        scrollable(body).height(Length::Fill).into()
    }

    /* ------------------------------------------------------------------ */
    /*                               queue                                */
    /* ------------------------------------------------------------------ */

    fn queue_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let player = &view.player;
        let mut tabs = Row::new().spacing(4);
        for key in &player.queue_keys {
            let active = key == &player.queue_key;
            tabs = tabs.push(
                button(text(key.clone()).size(12))
                    .padding([4, 8])
                    .style(if active {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(UiMessage::User(backend::Message::ActivateQueue(
                        key.clone(),
                    ))),
            );
        }
        let header = row![
            tabs,
            space().width(Length::Fill),
            button("清空队列")
                .padding([3, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::ClearQueue)),
        ]
        .align_y(Alignment::Center);

        if player.queue.is_empty() {
            return column![
                header,
                empty(theme, "队列是空的：在任意列表里点一首歌开始播放")
            ]
            .spacing(8)
            .into();
        }

        let mut rows = Column::new().spacing(2);
        for (index, song) in player.queue.iter().enumerate() {
            let current = player.queue_index == Some(index);
            let title = format!(
                "{} {:>3}. {}  ·  {}",
                if current { "▶" } else { " " },
                index + 1,
                song.name,
                song.singer
            );
            rows = rows.push(
                button(
                    row![
                        text(title).size(13).width(Length::Fill),
                        text(format_duration(song.duration))
                            .size(11)
                            .color(muted(theme)),
                    ]
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([5, 8])
                .style(if current {
                    button::primary
                } else {
                    button::text
                })
                .on_press(UiMessage::User(backend::Message::PlayQueueRow(index))),
            );
        }
        scrollable(column![header, rows].spacing(6))
            .height(Length::Fill)
            .into()
    }

    /* ------------------------------------------------------------------ */
    /*                              lyrics                                */
    /* ------------------------------------------------------------------ */

    fn lyrics_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let player = &view.player;
        if player.song.is_none() {
            return empty(theme, "播放一首歌后这里显示歌词");
        }
        if player.lyrics.is_empty() {
            return empty(theme, "这首歌暂时没有歌词");
        }
        let active = current_lyric_index(&player.lyrics, player.position_secs);
        let mut lines = Column::new().spacing(5).padding([8, 18]);
        for (index, line) in player.lyrics.iter().enumerate() {
            let is_active = active == Some(index);
            lines = lines.push(
                text(line.text.clone())
                    .size(if is_active { 21 } else { 15 })
                    .color(if is_active {
                        theme.palette().primary
                    } else {
                        muted(theme)
                    }),
            );
            if view.settings.lyric_translation
                && let Some(translation) = translation_at(&player.translated_lyrics, line)
            {
                lines = lines.push(
                    text(translation.to_string())
                        .size(if is_active { 15 } else { 12 })
                        .color(muted(theme)),
                );
            }
        }
        let lyrics = scrollable(lines).height(Length::Fill);
        if let Some(info) = &player.mv {
            let mut panel = Column::new().spacing(6).width(Length::Fixed(300.0));
            if let Some(handle) = self.mv() {
                panel = panel.push(
                    image(handle.clone())
                        .width(Length::Fill)
                        .height(Length::Fixed(260.0))
                        .content_fit(ContentFit::Contain),
                );
            }
            panel = panel
                .push(text(info.name.clone()).size(16))
                .push(text(format!("{} · {}", info.artist_name, info.publish_time)).size(12))
                .push(text(info.desc.clone()).size(12).color(muted(theme)));
            return row![lyrics, scrollable(panel).height(Length::Fill)]
                .spacing(12)
                .into();
        }
        lyrics.into()
    }

    /* ------------------------------------------------------------------ */
    /*                               login                                */
    /* ------------------------------------------------------------------ */

    fn login_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let login = &view.login;
        let mut body = Column::new().spacing(12);

        if let Some(user) = &view.user {
            body = body.push(
                column![
                    text(format!("已登录：{}", user.nickname)).size(20),
                    text(format!("UID {}", user.uid))
                        .size(11)
                        .color(muted(theme)),
                ]
                .spacing(4),
            );
        } else {
            body = body.push(text("登录网易云音乐").size(20));
        }

        let mut tabs = Row::new().spacing(6);
        for method in LoginMethod::ALL {
            tabs = tabs.push(
                button(text(method.label()).size(13))
                    .padding([5, 12])
                    .style(if login.method == method {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(UiMessage::User(backend::Message::SetLoginMethod(method))),
            );
        }
        body = body.push(tabs);

        match login.method {
            LoginMethod::Qr => {
                if view.user.is_none() {
                    if let Some(qr) = self.qr() {
                        body = body.push(
                            container(qr_code(qr).cell_size(5).total_size(250))
                                .padding(10)
                                .style(|_theme: &Theme| container::Style {
                                    background: Some(Color::WHITE.into()),
                                    border: Border {
                                        radius: 8.0.into(),
                                        ..Border::default()
                                    },
                                    ..container::Style::default()
                                }),
                        );
                    } else {
                        body = body.push(
                            text(if login.loading {
                                "正在生成二维码…"
                            } else {
                                "二维码还没有生成"
                            })
                            .size(13)
                            .color(muted(theme)),
                        );
                    }
                    body = body.push(text(login.qr_status.clone()).size(13).color(muted(theme)));
                    body = body.push(
                        button("刷新二维码").on_press(UiMessage::User(backend::Message::RefreshQr)),
                    );
                }
            }
            LoginMethod::Password => {
                body = body.push(
                    text_input("手机号 / 邮箱", &login.account)
                        .on_input(|value| {
                            UiMessage::User(backend::Message::SetLoginField(
                                LoginField::Account,
                                value,
                            ))
                        })
                        .on_submit(UiMessage::User(backend::Message::SubmitLogin))
                        .padding(8)
                        .width(Length::Fixed(420.0)),
                );
                body = body.push(
                    text_input("密码", &login.password)
                        .secure(true)
                        .on_input(|value| {
                            UiMessage::User(backend::Message::SetLoginField(
                                LoginField::Password,
                                value,
                            ))
                        })
                        .on_submit(UiMessage::User(backend::Message::SubmitLogin))
                        .padding(8)
                        .width(Length::Fixed(420.0)),
                );
                body = body.push(
                    button("登录")
                        .padding([6, 18])
                        .style(button::primary)
                        .on_press(UiMessage::User(backend::Message::SubmitLogin)),
                );
            }
            LoginMethod::Sms => {
                body = body.push(
                    text_input("手机号", &login.phone)
                        .on_input(|value| {
                            UiMessage::User(backend::Message::SetLoginField(
                                LoginField::Phone,
                                value,
                            ))
                        })
                        .padding(8)
                        .width(Length::Fixed(420.0)),
                );
                body = body.push(
                    row![
                        button("发送验证码")
                            .padding([5, 12])
                            .on_press(UiMessage::User(backend::Message::SendSmsCode)),
                        text("验证码会发送到上面的手机号")
                            .size(11)
                            .color(muted(theme)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
                body = body.push(
                    text_input("验证码", &login.code)
                        .on_input(|value| {
                            UiMessage::User(backend::Message::SetLoginField(
                                LoginField::Code,
                                value,
                            ))
                        })
                        .on_submit(UiMessage::User(backend::Message::SubmitLogin))
                        .padding(8)
                        .width(Length::Fixed(420.0)),
                );
                body = body.push(
                    button("登录")
                        .padding([6, 18])
                        .style(button::primary)
                        .on_press(UiMessage::User(backend::Message::SubmitLogin)),
                );
            }
            LoginMethod::Sign => {
                body = body.push(
                    button("每日签到")
                        .padding([6, 18])
                        .on_press(UiMessage::User(backend::Message::DailySign)),
                );
            }
        }

        if let Some(error) = &login.error {
            body = body.push(text(error.clone()).size(12).color(theme.palette().danger));
        }
        if let Some(notice) = &login.notice {
            body = body.push(text(notice.clone()).size(12).color(theme.palette().success));
        }
        if view.user.is_some() {
            body = body.push(
                button("退出登录")
                    .padding([5, 12])
                    .style(button::text)
                    .on_press(UiMessage::User(backend::Message::Logout)),
            );
        }

        scrollable(body).height(Length::Fill).into()
    }

    /* ------------------------------------------------------------------ */
    /*                              settings                              */
    /* ------------------------------------------------------------------ */

    fn settings_page<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let settings = &view.settings;
        let mut body = Column::new().spacing(8);
        body = body.push(text("设置").size(22));

        body = body.push(setting_row(
            "主题",
            &settings.theme,
            UiMessage::User(backend::Message::CycleTheme),
            theme,
        ));
        body = body.push(setting_row(
            "音质（下一首生效）",
            &settings.quality,
            UiMessage::User(backend::Message::CycleQuality),
            theme,
        ));
        body = body.push(setting_row(
            "边听边存",
            if settings.save_on_play { "开" } else { "关" },
            UiMessage::User(backend::Message::ToggleSaveOnPlay),
            theme,
        ));
        body = body.push(setting_row(
            "切歌通知",
            if settings.notify_song_change {
                "开"
            } else {
                "关"
            },
            UiMessage::User(backend::Message::ToggleNotifySongChange),
            theme,
        ));
        body = body.push(setting_row(
            "错误通知",
            if settings.notify_errors { "开" } else { "关" },
            UiMessage::User(backend::Message::ToggleNotifyErrors),
            theme,
        ));
        body = body.push(setting_row(
            "歌词译文",
            if settings.lyric_translation {
                "开"
            } else {
                "关"
            },
            UiMessage::User(backend::Message::ToggleTranslation),
            theme,
        ));
        body = body.push(setting_row(
            "频谱",
            if settings.visualizer { "开" } else { "关" },
            UiMessage::User(backend::Message::ToggleVisualizer),
            theme,
        ));
        body = body.push(setting_row(
            "音高读数",
            if settings.pitch { "开" } else { "关" },
            UiMessage::User(backend::Message::TogglePitch),
            theme,
        ));
        body = body.push(
            text("音质从下一首开始生效；设置写入 config.toml。")
                .size(11)
                .color(muted(theme)),
        );
        scrollable(body).height(Length::Fill).into()
    }

    /* ------------------------------------------------------------------ */
    /*                            player bar                              */
    /* ------------------------------------------------------------------ */

    fn player_bar<'a>(&'a self, view: &'a View, theme: &Theme) -> Element<'a, UiMessage> {
        let player = &view.player;
        let song = player.song.as_ref();
        let title = song
            .map(|song| song.name.clone())
            .unwrap_or_else(|| "未在播放".into());
        let artist = song
            .map(|song| format!("{} · {}", song.singer, song.album))
            .unwrap_or_default();
        let duration = player.duration_secs.max(0.001);
        let position = self
            .seek_preview()
            .unwrap_or(player.position_secs)
            .clamp(0.0, duration);

        let art: Element<'_, UiMessage> = match self.cover() {
            Some(handle) => image::Image::new(handle.clone())
                .width(Length::Fixed(64.0))
                .height(Length::Fixed(64.0))
                .content_fit(ContentFit::Cover)
                .border_radius(8.0)
                .into(),
            None => container(text("♪").size(26))
                .width(Length::Fixed(64.0))
                .height(Length::Fixed(64.0))
                .center(Length::Fixed(64.0))
                .style(|theme: &Theme| container::Style {
                    background: Some(theme.extended_palette().background.strong.color.into()),
                    border: Border {
                        radius: 8.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                })
                .into(),
        };

        let play_label = if player.playing && !player.paused {
            "⏸ 暂停"
        } else {
            "▶ 播放"
        };
        let like_label = if player.liked {
            "♥ 已喜欢"
        } else {
            "♡ 喜欢"
        };
        let (_, mode) = mode_icon(&player.mode);
        let controls = row![
            button("⏮")
                .padding([4, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::Prev)),
            button(play_label)
                .padding([4, 14])
                .style(button::primary)
                .on_press(UiMessage::User(backend::Message::TogglePlay)),
            button("⏭")
                .padding([4, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::Next)),
            button(text(format!("{} {mode}", mode_icon(&player.mode).0)))
                .padding([4, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::CycleMode)),
            button(like_label)
                .padding([4, 10])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::ToggleLike)),
        ]
        .spacing(5)
        .align_y(Alignment::Center);

        let mut info = column![
            text(title).size(15),
            text(artist).size(11).color(muted(theme)),
        ];
        if view.settings.pitch
            && let Some(pitch) = &player.pitch
        {
            info = info.push(text(pitch.clone()).size(10).color(muted(theme)));
        }
        let info = info.push(controls).spacing(3).width(Length::Fill);

        let progress = slider(0.0..=duration, position, |value| {
            UiMessage::Local(Local::SeekPreview(value))
        })
        .on_release(UiMessage::Local(Local::SeekCommit))
        .width(Length::Fill);
        let time_row = row![
            text(format_duration((position * 1000.0) as u64))
                .size(10)
                .color(muted(theme)),
            progress,
            text(format_duration((duration * 1000.0) as u64))
                .size(10)
                .color(muted(theme)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let volume = row![
            text("音量").size(10).color(muted(theme)),
            slider(0.0..=1.0, player.volume as f32, |value| {
                UiMessage::User(backend::Message::SetVolume(f64::from(value)))
            })
            .width(Length::Fixed(100.0)),
        ]
        .spacing(5)
        .align_y(Alignment::Center);

        let mut actions = row![
            button("队列")
                .padding([3, 8])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::Navigate(Page::Queue))),
            button("歌词")
                .padding([3, 8])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::Navigate(Page::Lyrics))),
            button("相似")
                .padding([3, 8])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::OpenSongContext(true))),
            button("相关歌单")
                .padding([3, 8])
                .style(button::text)
                .on_press(UiMessage::User(backend::Message::OpenSongContext(false))),
        ]
        .spacing(4);
        if player.cached {
            actions = actions.push(text("已缓存").size(10).color(theme.palette().success));
        }

        let mut bar = Column::new().spacing(5).push(
            row![art, info, actions, volume]
                .spacing(14)
                .align_y(Alignment::Center),
        );
        bar = bar.push(time_row);
        if view.settings.visualizer && !player.visualizer.is_empty() {
            bar = bar.push(spectrum(&player.visualizer, theme));
        }

        container(bar.padding(10))
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.weak.color.into()),
                border: Border {
                    radius: 10.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }
}

fn spectrum<'a>(values: &[f32], theme: &Theme) -> Element<'a, UiMessage> {
    let color = theme.palette().primary.scale_alpha(0.65);
    let mut bars = Row::new()
        .spacing(2)
        .height(Length::Fixed(18.0))
        .align_y(Alignment::End);
    for value in values.iter().take(64) {
        let height = (value.clamp(0.0, 1.0) * 18.0).max(1.0);
        bars = bars.push(
            container(space())
                .width(Length::Fill)
                .height(Length::Fixed(height))
                .style(move |_| container::Style {
                    background: Some(color.into()),
                    ..container::Style::default()
                }),
        );
    }
    bars.into()
}

fn setting_row<'a>(
    label: &'a str,
    value: &'a str,
    message: UiMessage,
    theme: &Theme,
) -> Element<'a, UiMessage> {
    row![
        text(label).size(14).width(Length::Fixed(140.0)),
        text(value).size(13).color(muted(theme)).width(Length::Fill),
        button("切换").padding([3, 10]).on_press(message),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn empty<'a>(theme: &Theme, message: &'a str) -> Element<'a, UiMessage> {
    container(text(message).size(13).color(muted(theme)))
        .padding(12)
        .into()
}

fn muted(theme: &Theme) -> Color {
    theme.extended_palette().background.weak.text
}
