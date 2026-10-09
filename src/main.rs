use litchee::LichessClient;
use litchee::api::broadcasting::tv::{LichessTvFeedEvent, LichessTvFeedPlayer};
use litchee::api::gameplay::board::LichessBoardEvent;
use litchee::model::LichessColor;

use shakmaty::Position;

use rodio::{Decoder, MixerDeviceSink};

use iced::futures::channel::mpsc;
use iced::futures::sink::SinkExt;

use iced::advanced::svg::{Handle, Svg};
use iced::widget::{Action, button, canvas, center, column, container, row, space, text};
use iced::{Alignment, Point, Rectangle, Renderer, Size, Subscription, Theme, mouse};

use std::str::FromStr;

mod auth;

const MOVE_SOUND: &[u8] = include_bytes!("../assets/sounds/Move.ogg");
const CAPTURE_SOUND: &[u8] = include_bytes!("../assets/sounds/Capture.ogg");

// White pieces
const WK: &[u8] = include_bytes!("../assets/pieces/wK.svg");
const WQ: &[u8] = include_bytes!("../assets/pieces/wQ.svg");
const WB: &[u8] = include_bytes!("../assets/pieces/wB.svg");
const WN: &[u8] = include_bytes!("../assets/pieces/wN.svg");
const WR: &[u8] = include_bytes!("../assets/pieces/wR.svg");
const WP: &[u8] = include_bytes!("../assets/pieces/wP.svg");

// Black pieces
const BK: &[u8] = include_bytes!("../assets/pieces/bK.svg");
const BQ: &[u8] = include_bytes!("../assets/pieces/bQ.svg");
const BB: &[u8] = include_bytes!("../assets/pieces/bB.svg");
const BN: &[u8] = include_bytes!("../assets/pieces/bN.svg");
const BR: &[u8] = include_bytes!("../assets/pieces/bR.svg");
const BP: &[u8] = include_bytes!("../assets/pieces/bP.svg");

// Colors
const WHITE_SQUARE_COLOR: iced::Color = iced::Color::from_rgb8(238, 238, 210);
const BLACK_SQUARE_COLOR: iced::Color = iced::Color::from_rgb8(118, 150, 86);

const SQUARE_SIZE: u32 = 80;

struct App {
    username: String,
    screen: Screen,
    _sink_handle: MixerDeviceSink,
    audio_player: rodio::Player,
    lichess_token: Option<String>,
    conn_tx: Option<mpsc::Sender<ConnInput>>,
}

#[derive(Default)]
struct Game {
    history: Vec<String>, // all moves so far, in UCI
    interactive: bool,
    fen: String,
    position: shakmaty::Chess,
    last_move: String,
    last_move_source: Option<shakmaty::Square>,
    last_move_dest: Option<shakmaty::Square>,
    white_clock: i64,
    black_clock: i64,
    orientation: LichessColor,
    players: Vec<LichessTvFeedPlayer>,
}

enum Screen {
    Menu,
    Watch(Game),
    Play(Game),
}

#[allow(dead_code)]
#[derive(Clone)]
enum Message {
    SetUsername(String),
    Connected(mpsc::Sender<ConnInput>),
    Authenticated(String),
    TvEvent(LichessTvFeedEvent),
    Tick(iced::time::Instant),
    Menu,
    Watch,
    Play,
    Login,
    ChessMove([shakmaty::Square; 2]),
    BoardEvent(LichessBoardEvent),
}

#[derive(Debug)]
enum ConnInput {
    Token(String),
    Menu,
    Login,
    Watch,
    Play,
    ChessMove(String),
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .centered()
        .subscription(App::subscription)
        .run()
}

impl App {
    fn new() -> Self {
        let mut sink_handle =
            rodio::DeviceSinkBuilder::open_default_sink().expect("open default audio stream");
        sink_handle.log_on_drop(false);
        let audio_player = rodio::Player::connect_new(&sink_handle.mixer());

        App {
            username: String::new(),
            screen: Screen::Menu,
            _sink_handle: sink_handle,
            audio_player,
            lichess_token: load_token(),
            conn_tx: None,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Connected(mut tx) => {
                if let Some(token) = &self.lichess_token {
                    let _ = tx.try_send(ConnInput::Token(token.clone()));
                }
                self.conn_tx = Some(tx);
            }
            Message::Authenticated(lichess_token) => {
                if let Err(err) = save_token(&lichess_token) {
                    eprintln!("{}", err);
                }
                self.lichess_token = Some(lichess_token);
            }
            Message::SetUsername(username) => {
                self.username = username;
            }
            Message::Menu => {
                if let Some(ref mut tx) = self.conn_tx {
                    let _ = tx.try_send(ConnInput::Menu);
                }
                self.screen = Screen::Menu;
            }
            Message::Login => {
                if let Some(ref mut tx) = self.conn_tx {
                    let _ = tx.try_send(ConnInput::Login);
                }
            }
            Message::Watch => {
                if let Some(ref mut tx) = self.conn_tx {
                    let _ = tx.try_send(ConnInput::Watch);
                }
                self.screen = Screen::Watch(Game::default());
            }
            Message::Play => {
                if let Some(ref mut tx) = self.conn_tx {
                    let _ = tx.try_send(ConnInput::Play);
                }
                let mut game = Game::default();
                game.interactive = true;
                self.screen = Screen::Play(game);
            }
            Message::BoardEvent(event) => match self.screen {
                Screen::Play(ref mut game) => match event {
                    LichessBoardEvent::OpponentGone(_opponent_gone) => {}
                    LichessBoardEvent::GameFull(game_full) => {
                        println!("{:?}", game_full);
                        if let Some(black_player) = game_full.black {
                            if let Some(name) = black_player.name {
                                if name == self.username {
                                    game.orientation = LichessColor::Black;
                                }
                            }
                        }
                    }
                    LichessBoardEvent::GameState(game_state) => {
                        println!("{:?}", game_state);
                        game.white_clock = game_state.wtime;
                        game.black_clock = game_state.btime;
                        let moves: Vec<String> = game_state
                            .moves
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect();
                        if moves.len() == game.history.len() + 1 {
                            // make opponent's move
                            if let Some(opponent_move) = moves.last() {
                                game.history.push(opponent_move.to_string());
                                if let Ok(uci) = opponent_move.parse::<shakmaty::uci::UciMove>() {
                                    if let Ok(legal_move) = uci.to_move(&game.position) {
                                        if legal_move.is_capture() {
                                            if let Ok(source) =
                                                Decoder::new(std::io::Cursor::new(CAPTURE_SOUND))
                                            {
                                                self.audio_player.stop();
                                                self.audio_player.append(source);
                                            }
                                        } else {
                                            if let Ok(source) =
                                                Decoder::new(std::io::Cursor::new(MOVE_SOUND))
                                            {
                                                self.audio_player.stop();
                                                self.audio_player.append(source);
                                            }
                                        }
                                        if let Ok(new_pos) = game.position.clone().play(legal_move)
                                        {
                                            game.position = new_pos;
                                        }
                                    }
                                }
                            }
                        } else if moves.len() == game.history.len() {
                            // validate my last move
                            if let Some(server_last) = moves.last() {
                                if let Some(local_last) = game.history.last() {
                                    if server_last != local_last {
                                        // reconcile board state
                                    }
                                }
                            }
                        } else {
                            // reconcile board state
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            Message::Tick(_) => match self.screen {
                Screen::Watch(ref mut game) => game.tick_clock(),
                Screen::Play(ref mut game) => game.tick_clock(),
                _ => {}
            },
            Message::ChessMove([from_square, to_square]) => match self.screen {
                Screen::Play(ref mut game) => {
                    let uci_move = shakmaty::uci::UciMove::Normal {
                        from: from_square,
                        to: to_square,
                        promotion: None,
                    };
                    if let Ok(legal_move) = uci_move.to_move(&game.position) {
                        // send move as uci string to ConnInput
                        if let Some(ref mut tx) = self.conn_tx {
                            let _ = tx.try_send(ConnInput::ChessMove(uci_move.to_string()));
                        }
                        if legal_move.is_capture() {
                            if let Ok(source) = Decoder::new(std::io::Cursor::new(CAPTURE_SOUND)) {
                                self.audio_player.stop();
                                self.audio_player.append(source);
                            }
                        } else {
                            if let Ok(source) = Decoder::new(std::io::Cursor::new(MOVE_SOUND)) {
                                self.audio_player.stop();
                                self.audio_player.append(source);
                            }
                        }
                        if let Ok(new_pos) = game.position.clone().play(legal_move) {
                            game.position = new_pos;
                            game.history.push(uci_move.to_string());
                        }
                    }
                }
                _ => {}
            },
            Message::TvEvent(event) => match self.screen {
                Screen::Watch(ref mut game) => match event {
                    LichessTvFeedEvent::Fen(data) => {
                        let mut sound = MOVE_SOUND;
                        game.fen = data.fen;
                        game.white_clock = data.wc as i64;
                        game.black_clock = data.bc as i64;
                        game.last_move = data.lm;

                        if let Ok(uci) = game.last_move.parse::<shakmaty::uci::UciMove>() {
                            if let Ok(chess_move) = uci.to_move(&game.position) {
                                game.last_move_source = chess_move.from();
                                game.last_move_dest = Some(chess_move.to());
                                if chess_move.is_capture() {
                                    sound = CAPTURE_SOUND;
                                }
                                game.position.play_unchecked(chess_move);
                            }
                        }

                        if let Ok(source) = Decoder::new(std::io::Cursor::new(sound)) {
                            self.audio_player.stop();
                            self.audio_player.append(source);
                        }
                    }
                    LichessTvFeedEvent::Featured(data) => {
                        *game = Game::default();
                        game.orientation = data.orientation;
                        for player in &data.players {
                            match player.color {
                                LichessColor::White => {
                                    game.white_clock = player.seconds as i64;
                                }
                                LichessColor::Black => {
                                    game.black_clock = player.seconds as i64;
                                }
                            }
                        }
                        game.players = data.players;

                        let fen = shakmaty::fen::Fen::from_str(&data.fen).unwrap_or_default();
                        game.position = fen
                            .into_position(shakmaty::CastlingMode::Standard)
                            .unwrap_or_default();
                    }
                    _ => {}
                },
                _ => {}
            },
        }
    }

    fn view(&self) -> iced::Element<'_, Message> {
        match &self.screen {
            Screen::Menu => {
                let play_button = if self.lichess_token.is_some() {
                    button("Play").on_press(Message::Play)
                } else {
                    button("Login").on_press(Message::Login)
                };
                center(
                    column![
                        text("Lichess Iced").size(80),
                        space().height(20),
                        play_button,
                        button("Watch").on_press(Message::Watch),
                    ]
                    .spacing(20)
                    .align_x(Alignment::Center),
                )
                .into()
            }
            Screen::Watch(game) => {
                let mut top_player = String::new();
                let mut top_player_time = String::new();
                let mut bottom_player = String::new();
                let mut bottom_player_time = String::new();

                for player in &game.players {
                    if let Some(ref user) = player.user {
                        if player.color != game.orientation {
                            // top player
                            top_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                top_player_time = seconds_to_clock(game.white_clock);
                            } else {
                                top_player_time = seconds_to_clock(game.black_clock);
                            }
                        } else {
                            // bottom player
                            bottom_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                bottom_player_time = seconds_to_clock(game.white_clock);
                            } else {
                                bottom_player_time = seconds_to_clock(game.black_clock);
                            }
                        }
                    }
                }
                center(column![
                    container(row![button("Menu").on_press(Message::Menu)])
                        .width(640)
                        .padding(5),
                    container(row![
                        text(top_player),
                        space::horizontal(),
                        text(top_player_time)
                    ])
                    .width(640)
                    .padding(5),
                    canvas(game).height(SQUARE_SIZE * 8).width(SQUARE_SIZE * 8),
                    container(row![
                        text(bottom_player),
                        space::horizontal(),
                        text(bottom_player_time)
                    ])
                    .width(640)
                    .padding(5),
                ])
                .into()
            }
            Screen::Play(game) => {
                let mut top_player = String::new();
                let mut top_player_time = String::new();
                let mut bottom_player = String::new();
                let mut bottom_player_time = String::new();

                for player in &game.players {
                    if let Some(ref user) = player.user {
                        if player.color != game.orientation {
                            // top player
                            top_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                top_player_time = milliseconds_to_clock(game.white_clock);
                            } else {
                                top_player_time = milliseconds_to_clock(game.black_clock);
                            }
                        } else {
                            // bottom player
                            bottom_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                bottom_player_time = milliseconds_to_clock(game.white_clock);
                            } else {
                                bottom_player_time = milliseconds_to_clock(game.black_clock);
                            }
                        }
                    }
                }

                center(column![
                    container(row![button("Menu").on_press(Message::Menu)])
                        .width(640)
                        .padding(5),
                    container(row![
                        text(top_player),
                        space::horizontal(),
                        text(top_player_time)
                    ])
                    .width(640)
                    .padding(5),
                    canvas(game).height(SQUARE_SIZE * 8).width(SQUARE_SIZE * 8),
                    container(row![
                        text(bottom_player),
                        space::horizontal(),
                        text(bottom_player_time)
                    ])
                    .width(640)
                    .padding(5),
                ])
                .into()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(lichess_conn),
            iced::time::every(iced::time::Duration::from_secs(1)).map(Message::Tick),
        ])
    }
}

fn lichess_conn() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(100, async |mut output| {
        let mut client = LichessClient::new();

        let (sender, mut receiver) = mpsc::channel(100);
        let _ = output.send(Message::Connected(sender)).await;

        let mut feed_task = None;
        let mut game_id = None;

        loop {
            use iced_futures::futures::StreamExt;

            let input = receiver.select_next_some().await;

            match input {
                ConnInput::Token(secret) => {
                    if let Ok(c) = LichessClient::builder().token(secret).build() {
                        client = c;
                    }
                }
                ConnInput::Login => {
                    let client_id = std::env::var("LICHESS_CLIENT_ID")
                        .unwrap_or_else(|_| crate::auth::DEFAULT_CLIENT_ID.to_owned());

                    // An unauthenticated client is enough to build the authorization URL.
                    if let Some(token) = crate::auth::run_login(&client, &client_id).await.ok() {
                        let secret = token.access_token.into_inner();
                        let _ = output.send(Message::Authenticated(secret.clone())).await;
                        // Re-build the client, this time carrying the bearer token.
                        if let Ok(c) = LichessClient::builder().token(secret).build() {
                            client = c;
                        }
                    }

                    if let Ok(me) = client.account().profile().await {
                        let _ = output
                            .send(Message::SetUsername(me.user.username.clone()))
                            .await;
                        println!("\n✅ Signed in as {} ({})\n", me.user.username, me.url);
                        let _ = crate::auth::show_recent_games(&client, &me.user.username).await;
                        let _ = crate::auth::show_recent_puzzles(&client).await;
                        let _ = crate::auth::show_studies(&client, &me.user.username).await;
                    }
                }
                ConnInput::Watch => {
                    if let Ok(mut feed) = client.tv().feed().await {
                        // spawn new task for tv feed
                        let mut out_clone = output.clone();
                        feed_task = Some(tokio::spawn(async move {
                            while let Some(Ok(event)) = feed.next().await {
                                let _ = out_clone.send(Message::TvEvent(event)).await;
                            }
                        }));
                    }
                }
                ConnInput::Play => {
                    if let Ok(lichess_game) = client.challenges().challenge_ai(1).send().await {
                        game_id = Some(lichess_game.id.clone());
                        if let Ok(mut feed) = client.board().stream_game(&lichess_game.id).await {
                            let mut out_clone = output.clone();
                            feed_task = Some(tokio::spawn(async move {
                                while let Some(Ok(event)) = feed.next().await {
                                    let _ = out_clone.send(Message::BoardEvent(event)).await;
                                }
                            }));
                        }
                    }
                }
                ConnInput::ChessMove(chess_move) => {
                    println!("{:?}", chess_move);
                    if let Some(id) = &game_id {
                        let _ = client.board().make_move(&id, &chess_move, false);
                    }
                }
                ConnInput::Menu => {
                    if let Some(task) = feed_task.take() {
                        task.abort();
                        game_id = None;
                    }
                }
            }
        }
    })
}

impl Game {
    fn tick_clock(&mut self) {
        match self.position.turn() {
            shakmaty::Color::White => {
                if self.white_clock > 0 {
                    self.white_clock -= 1;
                }
            }
            shakmaty::Color::Black => {
                if self.black_clock > 0 {
                    self.black_clock -= 1;
                }
            }
        };
    }

    fn get_square(&self, x: u32, y: u32) -> shakmaty::Square {
        match self.orientation {
            LichessColor::White => {
                shakmaty::Square::from_coords(shakmaty::File::new(x), shakmaty::Rank::new(7 - y))
            }
            LichessColor::Black => {
                shakmaty::Square::from_coords(shakmaty::File::new(7 - x), shakmaty::Rank::new(y))
            }
        }
    }
}

#[derive(Default)]
struct GameProgramState {
    hovering_piece: Option<shakmaty::Piece>,
    dragging_piece: Option<shakmaty::Piece>,
    drag_top_left: Point,
    drag_from_square: Option<shakmaty::Square>,
    drag_to_square: Option<shakmaty::Square>,
}

impl canvas::Program<Message> for Game {
    type State = GameProgramState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        if !self.interactive {
            return None;
        }
        match event {
            iced::Event::Mouse(mouse_event) => match mouse_event {
                mouse::Event::CursorMoved { position: _ } => {
                    if let Some(point) = cursor.position_in(bounds) {
                        let x = (point.x / SQUARE_SIZE as f32).floor();
                        let y = (point.y / SQUARE_SIZE as f32).floor();
                        state.drag_top_left = Point::new(
                            point.x - (SQUARE_SIZE / 2) as f32,
                            point.y - (SQUARE_SIZE / 2) as f32,
                        );
                        // if already dragging, request repaint
                        if state.dragging_piece.is_some() {
                            return Some(Action::request_redraw());
                        }
                        let square = self.get_square(x as u32, y as u32);
                        if let Some(piece) = self.position.board().piece_at(square) {
                            state.hovering_piece = Some(piece);
                        } else {
                            state.hovering_piece = None;
                        }
                    }
                }
                mouse::Event::ButtonPressed(button) => match button {
                    mouse::Button::Left => {
                        if let Some(point) = cursor.position_in(bounds) {
                            let x = (point.x / SQUARE_SIZE as f32).floor();
                            let y = (point.y / SQUARE_SIZE as f32).floor();
                            state.drag_top_left = Point::new(
                                point.x - (SQUARE_SIZE / 2) as f32,
                                point.y - (SQUARE_SIZE / 2) as f32,
                            );
                            let square = self.get_square(x as u32, y as u32);
                            state.drag_from_square = Some(square);
                            if let Some(piece) = self.position.board().piece_at(square) {
                                state.dragging_piece = Some(piece);
                                state.hovering_piece = None;
                            }
                        }
                    }
                    _ => {}
                },
                mouse::Event::ButtonReleased(button) => match button {
                    mouse::Button::Left => {
                        state.dragging_piece = None;
                        if let Some(point) = cursor.position_in(bounds) {
                            let x = (point.x / SQUARE_SIZE as f32).floor();
                            let y = (point.y / SQUARE_SIZE as f32).floor();
                            state.drag_top_left = Point::new(
                                point.x - (SQUARE_SIZE / 2) as f32,
                                point.y - (SQUARE_SIZE / 2) as f32,
                            );
                            let square = self.get_square(x as u32, y as u32);
                            state.drag_to_square = Some(square);
                            if let Some(piece) = self.position.board().piece_at(square) {
                                state.hovering_piece = Some(piece);
                            } else {
                                state.hovering_piece = None;
                            }

                            if let Some(from_square) = state.drag_from_square {
                                if let Some(to_square) = state.drag_to_square {
                                    // send chess move message
                                    return Some(Action::publish(Message::ChessMove([
                                        from_square,
                                        to_square,
                                    ])));
                                }
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            _ => {}
        }

        None
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        // change cursor when hovering over piece
        if state.dragging_piece.is_some() {
            return iced::mouse::Interaction::Grabbing;
        }
        if state.hovering_piece.is_some() {
            return iced::mouse::Interaction::Grab;
        }
        iced::mouse::Interaction::default()
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let light_last_move_color = iced::Color::from_rgb8(180, 185, 190);
        let dark_last_move_color = iced::Color::from_rgb8(145, 150, 155);

        let square_size = Size::from([SQUARE_SIZE as f32, SQUARE_SIZE as f32]);

        let mut color;
        for file in 0..8 {
            for rank in 0..8 {
                let square = match self.orientation {
                    LichessColor::White => shakmaty::Square::from_coords(
                        shakmaty::File::new(file),
                        shakmaty::Rank::new(7 - rank),
                    ),
                    LichessColor::Black => shakmaty::Square::from_coords(
                        shakmaty::File::new(7 - file),
                        shakmaty::Rank::new(rank),
                    ),
                };

                let light_square = (file + rank) % 2 == 0;
                if light_square {
                    color = WHITE_SQUARE_COLOR;
                } else {
                    color = BLACK_SQUARE_COLOR;
                }

                let top_left = Point::new((file * SQUARE_SIZE) as f32, (rank * SQUARE_SIZE) as f32);
                let rect = canvas::Path::rectangle(top_left, square_size);

                // last move highlight
                if let Some(s) = self.last_move_source {
                    if s == square {
                        if light_square {
                            color = light_last_move_color;
                        } else {
                            color = dark_last_move_color;
                        };
                    }
                }
                if let Some(s) = self.last_move_dest {
                    if s == square {
                        if light_square {
                            color = light_last_move_color;
                        } else {
                            color = dark_last_move_color;
                        };
                    }
                }

                frame.fill(&rect, color);

                // prevent drawing duplicate piece while dragging
                if state.dragging_piece.is_some()
                    && let Some(from_square) = state.drag_from_square
                {
                    if from_square == square {
                        continue;
                    }
                }
                if let Some(piece) = self.position.board().piece_at(square) {
                    let svg = piece_to_svg(piece);
                    frame.draw_svg(Rectangle::new(top_left, square_size), svg);
                }
            }
        }

        // draw dragged piece
        if let Some(piece) = state.dragging_piece {
            let svg = piece_to_svg(piece);
            frame.draw_svg(Rectangle::new(state.drag_top_left, square_size), svg);
        }

        vec![frame.into_geometry()]
    }
}

fn piece_to_svg(piece: shakmaty::Piece) -> Svg {
    match (piece.color, piece.role) {
        (shakmaty::Color::White, shakmaty::Role::Pawn) => Svg::from(&Handle::from_memory(WP)),
        (shakmaty::Color::White, shakmaty::Role::Knight) => Svg::from(&Handle::from_memory(WN)),
        (shakmaty::Color::White, shakmaty::Role::Bishop) => Svg::from(&Handle::from_memory(WB)),
        (shakmaty::Color::White, shakmaty::Role::Rook) => Svg::from(&Handle::from_memory(WR)),
        (shakmaty::Color::White, shakmaty::Role::Queen) => Svg::from(&Handle::from_memory(WQ)),
        (shakmaty::Color::White, shakmaty::Role::King) => Svg::from(&Handle::from_memory(WK)),
        (shakmaty::Color::Black, shakmaty::Role::Pawn) => Svg::from(&Handle::from_memory(BP)),
        (shakmaty::Color::Black, shakmaty::Role::Knight) => Svg::from(&Handle::from_memory(BN)),
        (shakmaty::Color::Black, shakmaty::Role::Bishop) => Svg::from(&Handle::from_memory(BB)),
        (shakmaty::Color::Black, shakmaty::Role::Rook) => Svg::from(&Handle::from_memory(BR)),
        (shakmaty::Color::Black, shakmaty::Role::Queen) => Svg::from(&Handle::from_memory(BQ)),
        (shakmaty::Color::Black, shakmaty::Role::King) => Svg::from(&Handle::from_memory(BK)),
    }
}

fn seconds_to_clock(total_seconds: i64) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

fn milliseconds_to_clock(millis: i64) -> String {
    let total_seconds = millis / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let tenths = (millis % 1000) / 100;

    if minutes > 0 {
        format!("{:02}:{:02}", minutes, seconds)
    } else {
        format!("{:02}.{}", seconds, tenths)
    }
}

const SERVICE_NAME: &str = "lichess_iced";
const ACCOUNT_NAME: &str = "lichess_oauth_token";

fn save_token(token: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME)?;
    entry.set_password(token)
}

fn load_token() -> Option<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME).ok()?;
    entry.get_password().ok()
}

// fn delete_token() -> Result<(), keyring::Error> {
//     let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME)?;
//     entry.delete_password()
// }
