use litchee::LichessClient;
use litchee::api::broadcasting::tv::{LichessTvFeedEvent, LichessTvFeedPlayer};
use litchee::model::LichessColor;

use shakmaty::Position;

use rodio::{Decoder, MixerDeviceSink};

use iced::advanced::svg::{Handle, Svg};
use iced::futures::sink::SinkExt;
use iced::futures::stream::StreamExt;
use iced::widget::{canvas, center, column, container, row, space, text};
use iced::{Point, Rectangle, Renderer, Size, Subscription, Theme, mouse};

use std::io::Cursor;
use std::str::FromStr;

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
    screen: Screen,
    _sink_handle: MixerDeviceSink,
    audio_player: rodio::Player,
}

#[derive(Default)]
struct Game {
    fen: String,
    position: shakmaty::Chess,
    last_move: String,
    last_move_source: Option<shakmaty::Square>,
    last_move_dest: Option<shakmaty::Square>,
    white_clock: i32,
    black_clock: i32,
    orientation: LichessColor,
    players: Vec<LichessTvFeedPlayer>,
}

enum Screen {
    Tv(Game),
}

#[allow(dead_code)]
#[derive(Clone)]
enum Message {
    TvEvent(LichessTvFeedEvent),
    Tick(iced::time::Instant),
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
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
            screen: Screen::Tv(Game::default()),
            _sink_handle: sink_handle,
            audio_player,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick(_) => match self.screen {
                Screen::Tv(ref mut tv_game) => match tv_game.position.turn() {
                    shakmaty::Color::White => {
                        if tv_game.white_clock > 0 {
                            tv_game.white_clock -= 1;
                        }
                    }
                    shakmaty::Color::Black => {
                        if tv_game.black_clock > 0 {
                            tv_game.black_clock -= 1;
                        }
                    }
                },
            },
            Message::TvEvent(event) => match self.screen {
                Screen::Tv(ref mut tv_game) => match event {
                    LichessTvFeedEvent::Fen(data) => {
                        let mut sound = MOVE_SOUND;
                        tv_game.fen = data.fen;
                        tv_game.white_clock = data.wc;
                        tv_game.black_clock = data.bc;
                        tv_game.last_move = data.lm;
                        // if let Ok(last_move) = shakmaty::Move::from_str(&tv_game.last_move) {
                        //     let source = last_move.get_source();
                        //     let dest = last_move.get_dest();
                        //     tv_game.last_move_source = Some(source);
                        //     tv_game.last_move_dest = Some(dest);
                        //     if is_capture(&tv_game.board, source, dest) {
                        //         sound = CAPTURE_SOUND;
                        //     }
                        //     let mut new_board = tv_game.board.clone();
                        //     tv_game.board.make_move(last_move, &mut new_board);
                        //     tv_game.board = new_board;
                        // }

                        if let Ok(uci) = tv_game.last_move.parse::<shakmaty::uci::UciMove>() {
                            if let Ok(chess_move) = uci.to_move(&tv_game.position) {
                                tv_game.last_move_source = chess_move.from();
                                tv_game.last_move_dest = Some(chess_move.to());
                                if chess_move.is_capture() {
                                    sound = CAPTURE_SOUND;
                                }
                                tv_game.position.play_unchecked(chess_move);
                            }
                        }

                        if let Ok(source) = Decoder::new(Cursor::new(sound)) {
                            self.audio_player.stop();
                            self.audio_player.append(source);
                        }
                    }
                    LichessTvFeedEvent::Featured(data) => {
                        *tv_game = Game::default();
                        tv_game.orientation = data.orientation;
                        for player in &data.players {
                            match player.color {
                                LichessColor::White => {
                                    tv_game.white_clock = player.seconds;
                                }
                                LichessColor::Black => {
                                    tv_game.black_clock = player.seconds;
                                }
                            }
                        }
                        tv_game.players = data.players;

                        let fen = shakmaty::fen::Fen::from_str(&data.fen).unwrap_or_default();
                        tv_game.position = fen
                            .into_position(shakmaty::CastlingMode::Standard)
                            .unwrap_or_default();
                    }
                    _ => {}
                },
            },
        }
    }

    fn view(&self) -> iced::Element<'_, Message> {
        match &self.screen {
            Screen::Tv(tv_game) => {
                let mut top_player = String::new();
                let mut top_player_time = String::new();
                let mut bottom_player = String::new();
                let mut bottom_player_time = String::new();

                for player in &tv_game.players {
                    if let Some(ref user) = player.user {
                        if player.color != tv_game.orientation {
                            // top player
                            top_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                top_player_time = seconds_to_clock(tv_game.white_clock);
                            } else {
                                top_player_time = seconds_to_clock(tv_game.black_clock);
                            }
                        } else {
                            // bottom player
                            bottom_player = format!("{} ({})", user.name, player.rating);
                            if player.color == LichessColor::White {
                                bottom_player_time = seconds_to_clock(tv_game.white_clock);
                            } else {
                                bottom_player_time = seconds_to_clock(tv_game.black_clock);
                            }
                        }
                    }
                }
                center(column![
                    container(row![
                        text(top_player),
                        space::horizontal(),
                        text(top_player_time)
                    ])
                    .width(640)
                    .padding(5),
                    canvas(tv_game)
                        .height(SQUARE_SIZE * 8)
                        .width(SQUARE_SIZE * 8),
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
        match self.screen {
            Screen::Tv(_) => Subscription::batch([
                Subscription::run(lichess_tv),
                iced::time::every(iced::time::Duration::from_secs(1)).map(Message::Tick),
            ]),
        }
    }
}

fn lichess_tv() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(100, async |mut output| {
        let client = LichessClient::new();
        if let Ok(mut feed) = client.tv().feed().await {
            while let Some(Ok(event)) = feed.next().await {
                let _ = output.send(Message::TvEvent(event)).await;
            }
        }
    })
}

impl<Message> canvas::Program<Message> for Game {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // validate last move
        let light_last_move_color = iced::Color::from_rgb8(180, 185, 190);
        let dark_last_move_color = iced::Color::from_rgb8(145, 150, 155);

        let mut color;
        for file in 0..8 {
            for rank in 0..8 {
                // let square = match self.orientation {
                //     LichessColor::White => chess::Square::make_square(
                //         chess::Rank::from_index(7 - rank),
                //         chess::File::from_index(file),
                //     ),
                //     LichessColor::Black => chess::Square::make_square(
                //         chess::Rank::from_index(rank),
                //         chess::File::from_index(7 - file),
                //     ),
                // };
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
                let size = Size::from([SQUARE_SIZE as f32, SQUARE_SIZE as f32]);
                let rect = canvas::Path::rectangle(top_left, size);

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

                if let Some(piece) = self.position.board().piece_at(square) {
                    let svg = piece_to_svg(piece);
                    frame.draw_svg(Rectangle::new(top_left, size), svg);
                }
            }
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

fn seconds_to_clock(total_seconds: i32) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}
