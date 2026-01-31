use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::{error::Error, io, time::Duration, fs, path::PathBuf};
use tokio::sync::mpsc;

#[derive(Deserialize, Debug, Clone)]
struct GeocodingResponse {
    results: Option<Vec<GeoLocation>>,
}

#[derive(Deserialize, Debug, Clone)]
struct GeoLocation {
    name: String,
    latitude: f64,
    longitude: f64,
    country: String,
    admin1: Option<String>,
}

#[derive(Deserialize, Debug)]
struct WeatherResponse {
    current: CurrentWeather,
    current_units: CurrentUnits,
}

#[derive(Deserialize, Debug)]
struct CurrentWeather {
    temperature_2m: f64,
    relative_humidity_2m: u32,
    apparent_temperature: f64,
    precipitation: f64,
    weather_code: u32,
    wind_speed_10m: f64,
    pressure_msl: f64,
}

#[derive(Deserialize, Debug)]
struct CurrentUnits {
    temperature_2m: String,
    wind_speed_10m: String,
    pressure_msl: String,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    Insert,
}

#[derive(PartialEq)]
enum AppState {
    Input,
    Loading,
    Display,
    Error,
}

#[derive(PartialEq)]
enum FocusedPane {
    Search,
    History,
}

struct WeatherData {
    location: GeoLocation,
    weather: WeatherResponse,
}

#[derive(Serialize, Deserialize, Clone)]
struct HistoryEntry {
    query: String,
    timestamp: u64,
}

struct App {
    input: String,
    cursor_position: usize,
    state: AppState,
    mode: Mode,
    weather_data: Option<WeatherData>,
    error_message: String,
    search_history: Vec<HistoryEntry>,
    autocomplete_suggestions: Vec<GeoLocation>,
    selected_suggestion: usize,
    show_autocomplete: bool,
    last_autocomplete_query: String,
    focused_pane: FocusedPane,
    selected_history_index: usize,
}

impl App {
    fn new() -> App {
        let history = load_history().unwrap_or_default();
        App {
            input: String::new(),
            cursor_position: 0,
            state: AppState::Input,
            mode: Mode::Normal,
            weather_data: None,
            error_message: String::new(),
            search_history: history,
            autocomplete_suggestions: Vec::new(),
            selected_suggestion: 0,
            show_autocomplete: false,
            last_autocomplete_query: String::new(),
            focused_pane: FocusedPane::Search,
            selected_history_index: 0,
        }
    }

   
    fn char_to_byte_pos(&self, char_pos: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_pos)
            .map(|(byte_pos, _)| byte_pos)
            .unwrap_or(self.input.len())
    }

    // Helper: get character count
    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_position < self.char_count() {
            self.cursor_position += 1;
        }
    }

    fn move_cursor_start(&mut self) {
        self.cursor_position = 0;
    }

    fn move_cursor_end(&mut self) {
        self.cursor_position = self.char_count();
    }

    fn delete_char(&mut self) {
        let char_count = self.char_count();
        if self.cursor_position < char_count {
            let byte_pos = self.char_to_byte_pos(self.cursor_position);
            self.input.remove(byte_pos);
        }
    }

    fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            let byte_pos = self.char_to_byte_pos(self.cursor_position);
            self.input.remove(byte_pos);
        }
    }

    fn insert_char(&mut self, c: char) {
        let byte_pos = self.char_to_byte_pos(self.cursor_position);
        self.input.insert(byte_pos, c);
        self.cursor_position += 1;
    }

    fn move_to_next_word(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let mut pos = self.cursor_position;
        
       
        while pos < chars.len() && !chars[pos].is_whitespace() {
            pos += 1;
        }
       
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        
        self.cursor_position = pos;
    }

    fn move_to_prev_word(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        
        let chars: Vec<char> = self.input.chars().collect();
        let mut pos = self.cursor_position.saturating_sub(1);
        
      
        while pos > 0 && chars[pos].is_whitespace() {
            pos -= 1;
        }
      
        while pos > 0 && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        
        self.cursor_position = pos;
    }

    fn select_next_suggestion(&mut self) {
        if !self.autocomplete_suggestions.is_empty() {
            self.selected_suggestion = 
                (self.selected_suggestion + 1) % self.autocomplete_suggestions.len();
        }
    }

    fn select_prev_suggestion(&mut self) {
        if !self.autocomplete_suggestions.is_empty() {
            if self.selected_suggestion == 0 {
                self.selected_suggestion = self.autocomplete_suggestions.len() - 1;
            } else {
                self.selected_suggestion -= 1;
            }
        }
    }

    fn accept_suggestion(&mut self) {
        if self.show_autocomplete && !self.autocomplete_suggestions.is_empty() {
            let suggestion = &self.autocomplete_suggestions[self.selected_suggestion];
            self.input = format!("{}, {}", suggestion.name, suggestion.country);
            self.cursor_position = self.char_count();
            self.show_autocomplete = false;
            self.autocomplete_suggestions.clear();
        }
    }

    fn select_next_history(&mut self) {
        if !self.search_history.is_empty() {
            self.selected_history_index = 
                (self.selected_history_index + 1) % self.search_history.len();
        }
    }

    fn select_prev_history(&mut self) {
        if !self.search_history.is_empty() {
            if self.selected_history_index == 0 {
                self.selected_history_index = self.search_history.len() - 1;
            } else {
                self.selected_history_index -= 1;
            }
        }
    }

    fn load_selected_history(&mut self) {
        if !self.search_history.is_empty() && self.selected_history_index < self.search_history.len() {
            self.input = self.search_history[self.selected_history_index].query.clone();
            self.cursor_position = self.char_count();
            self.focused_pane = FocusedPane::Search;
            self.mode = Mode::Insert;
        }
    }

    fn add_to_history(&mut self, query: String) {
        use std::time::{SystemTime, UNIX_EPOCH};
        
     
        if let Some(pos) = self.search_history.iter().position(|e| e.query == query) {
            self.search_history.remove(pos);
        }
        
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        self.search_history.insert(0, HistoryEntry { query, timestamp });
        
      
        if self.search_history.len() > 50 {
            self.search_history.truncate(50);
        }
        
        let _ = save_history(&self.search_history);
    }
}

enum AppMessage {
    AutocompleteResults(String, Vec<GeoLocation>),
}

fn get_history_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".weather_searcher_history.json");
    path
}

fn load_history() -> Result<Vec<HistoryEntry>, Box<dyn Error>> {
    let path = get_history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let history: Vec<HistoryEntry> = serde_json::from_str(&content)?;
    Ok(history)
}

fn save_history(history: &[HistoryEntry]) -> Result<(), Box<dyn Error>> {
    let path = get_history_path();
    let content = serde_json::to_string_pretty(history)?;
    fs::write(path, content)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::new();
    let res = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut pending_autocomplete: Option<String> = None;

    loop {
        terminal.draw(|f| ui(f, &app))?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                AppMessage::AutocompleteResults(query, suggestions) => {
                    if app.input == query {
                        app.autocomplete_suggestions = suggestions;
                        app.show_autocomplete = !app.autocomplete_suggestions.is_empty();
                        app.selected_suggestion = 0;
                    }
                }
            }
        }

        if crossterm::event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match app.state {
                    AppState::Input => {
                        match app.mode {
                            Mode::Normal => match key.code {
                                KeyCode::Char('i') => {
                                    app.mode = Mode::Insert;
                                }
                                KeyCode::Char('I') => {
                                    app.mode = Mode::Insert;
                                    app.move_cursor_start();
                                }
                                KeyCode::Char('a') => {
                                    app.mode = Mode::Insert;
                                    app.move_cursor_right();
                                }
                                KeyCode::Char('A') => {
                                    app.mode = Mode::Insert;
                                    app.move_cursor_end();
                                }
                                KeyCode::Char('h') => {
                                    if app.focused_pane == FocusedPane::Search {
                                        app.move_cursor_left();
                                    }
                                }
                                KeyCode::Char('l') => {
                                    if app.focused_pane == FocusedPane::Search {
                                        app.move_cursor_right();
                                    }
                                }
                                KeyCode::Char('j') => {
                                    if app.focused_pane == FocusedPane::History {
                                        app.select_next_history();
                                    }
                                }
                                KeyCode::Char('k') => {
                                    if app.focused_pane == FocusedPane::History {
                                        app.select_prev_history();
                                    }
                                }
                                KeyCode::Char('0') | KeyCode::Char('^') => app.move_cursor_start(),
                                KeyCode::Char('$') => app.move_cursor_end(),
                                KeyCode::Char('w') => app.move_to_next_word(),
                                KeyCode::Char('b') => app.move_to_prev_word(),
                                KeyCode::Char('x') => app.delete_char(),
                                KeyCode::Char('d') => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        app.input.clear();
                                        app.cursor_position = 0;
                                    }
                                }
                                KeyCode::Tab => {
                                    app.focused_pane = match app.focused_pane {
                                        FocusedPane::Search => FocusedPane::History,
                                        FocusedPane::History => FocusedPane::Search,
                                    };
                                }
                                KeyCode::Enter => {
                                    if app.focused_pane == FocusedPane::History {
                                        app.load_selected_history();
                                    } else if !app.input.is_empty() {
                                        let city = app.input.clone();
                                        app.state = AppState::Loading;
                                        terminal.draw(|f| ui(f, &app))?;

                                        match fetch_weather(&city).await {
                                            Ok(data) => {
                                                app.weather_data = Some(data);
                                                app.add_to_history(city);
                                                app.state = AppState::Display;
                                                app.input.clear();
                                                app.cursor_position = 0;
                                                app.mode = Mode::Normal;
                                            }
                                            Err(e) => {
                                                app.error_message = e;
                                                app.state = AppState::Error;
                                            }
                                        }
                                    }
                                }
                                KeyCode::Esc => {
                                    return Ok(());
                                }
                                _ => {}
                            },
                            Mode::Insert => match key.code {
                                KeyCode::Esc => {
                                    app.mode = Mode::Normal;
                                    if app.cursor_position > 0 {
                                        app.cursor_position -= 1;
                                    }
                                    app.show_autocomplete = false;
                                }
                                KeyCode::Char(c) => {
                                    app.insert_char(c);
                                    
                                    if app.input.len() >= 3 && app.input != app.last_autocomplete_query {
                                        pending_autocomplete = Some(app.input.clone());
                                        app.last_autocomplete_query = app.input.clone();
                                    }
                                }
                                KeyCode::Backspace => {
                                    app.backspace();
                                    if app.input.len() < 3 {
                                        app.show_autocomplete = false;
                                        app.autocomplete_suggestions.clear();
                                    } else if app.input != app.last_autocomplete_query {
                                        pending_autocomplete = Some(app.input.clone());
                                        app.last_autocomplete_query = app.input.clone();
                                    }
                                }
                                KeyCode::Delete => {
                                    app.delete_char();
                                }
                                KeyCode::Left => app.move_cursor_left(),
                                KeyCode::Right => app.move_cursor_right(),
                                KeyCode::Home => app.move_cursor_start(),
                                KeyCode::End => app.move_cursor_end(),
                                KeyCode::Down => {
                                    if app.show_autocomplete {
                                        app.select_next_suggestion();
                                    }
                                }
                                KeyCode::Up => {
                                    if app.show_autocomplete {
                                        app.select_prev_suggestion();
                                    }
                                }
                                KeyCode::Tab => {
                                    if app.show_autocomplete {
                                        app.accept_suggestion();
                                    } else {
                                        app.focused_pane = match app.focused_pane {
                                            FocusedPane::Search => FocusedPane::History,
                                            FocusedPane::History => FocusedPane::Search,
                                        };
                                    }
                                }
                                KeyCode::Enter => {
                                    if app.show_autocomplete && !app.autocomplete_suggestions.is_empty() {
                                        app.accept_suggestion();
                                    } else if !app.input.is_empty() {
                                        let city = app.input.clone();
                                        app.state = AppState::Loading;
                                        app.show_autocomplete = false;
                                        terminal.draw(|f| ui(f, &app))?;

                                        match fetch_weather(&city).await {
                                            Ok(data) => {
                                                app.weather_data = Some(data);
                                                app.add_to_history(city);
                                                app.state = AppState::Display;
                                                app.input.clear();
                                                app.cursor_position = 0;
                                                app.mode = Mode::Normal;
                                            }
                                            Err(e) => {
                                                app.error_message = e;
                                                app.state = AppState::Error;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            },
                        }
                    }
                    AppState::Display | AppState::Error => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            return Ok(());
                        }
                        KeyCode::Char('i') => {
                            app.state = AppState::Input;
                            app.mode = Mode::Insert;
                            app.error_message.clear();
                            app.show_autocomplete = false;
                        }
                        _ => {
                            app.state = AppState::Input;
                            app.mode = Mode::Normal;
                            app.error_message.clear();
                            app.show_autocomplete = false;
                        }
                    },
                    AppState::Loading => {}
                }
            }
        }

        if let Some(query) = pending_autocomplete.take() {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                if let Ok(suggestions) = fetch_autocomplete(&query).await {
                    let _ = tx_clone.send(AppMessage::AutocompleteResults(query, suggestions));
                }
            });
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    let mode_text = match app.mode {
        Mode::Normal => " -- NORMAL --",
        Mode::Insert => " -- INSERT --",
    };
    
    let title = Paragraph::new(format!("🌤  Weather TUI Search{}", mode_text))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[1]);

    match app.state {
        AppState::Input => {
            // Build the display string with cursor
            let chars: Vec<char> = app.input.chars().collect();
            let char_count = chars.len();
            
            let input_display = if app.cursor_position <= char_count {
                let before: String = chars.iter().take(app.cursor_position).collect();
                let after: String = chars.iter().skip(app.cursor_position).collect();
                
                if app.mode == Mode::Insert {
                    format!("{}█{}", before, after)
                } else {
                    if app.cursor_position < char_count {
                        let cursor_char = chars[app.cursor_position];
                        let after_cursor: String = chars.iter().skip(app.cursor_position + 1).collect();
                        format!("{}[{}]{}", before, cursor_char, after_cursor)
                    } else {
                        format!("{}█", before)
                    }
                }
            } else {
                app.input.clone()
            };

            let search_border_style = if app.focused_pane == FocusedPane::Search {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };

            let input = Paragraph::new(input_display)
                .style(Style::default().fg(Color::Yellow))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(search_border_style)
                        .title(format!("Search City (Mode: {})", 
                            if app.mode == Mode::Normal { "NORMAL" } else { "INSERT" })),
                );

            if app.show_autocomplete && !app.autocomplete_suggestions.is_empty() {
                let input_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(5)])
                    .split(main_chunks[0]);

                f.render_widget(input, input_chunks[0]);

                let suggestions: Vec<ListItem> = app
                    .autocomplete_suggestions
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let region_str = s.admin1.as_ref().map(|r| format!(", {}", r)).unwrap_or_default();
                        let content = format!("{}{} ({})", s.name, region_str, s.country);
                        let style = if i == app.selected_suggestion {
                            Style::default().fg(Color::Black).bg(Color::Yellow)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        ListItem::new(content).style(style)
                    })
                    .collect();

                let autocomplete = List::new(suggestions).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Suggestions (Up/Down to select, Tab to accept)"),
                );
                f.render_widget(autocomplete, input_chunks[1]);
            } else {
                f.render_widget(input, main_chunks[0]);
            }
        }
        AppState::Loading => {
            let loading = Paragraph::new("Loading weather data...")
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL).title("Status"));
            f.render_widget(loading, main_chunks[0]);
        }
        AppState::Display => {
            if let Some(data) = &app.weather_data {
                let weather_desc = weather_code_to_description(data.weather.current.weather_code);
                let region_str = data.location.admin1.as_ref()
                    .map(|r| format!(", {}", r))
                    .unwrap_or_default();

                let weather_text = vec![
                    Line::from(vec![
                        Span::styled("Location: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{}{} ({})", 
                                data.location.name,
                                region_str,
                                data.location.country),
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Condition: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            weather_desc,
                            Style::default().fg(Color::Yellow),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Temperature: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{:.1}{}", 
                                data.weather.current.temperature_2m,
                                data.weather.current_units.temperature_2m),
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Feels like: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{:.1}{}", 
                                data.weather.current.apparent_temperature,
                                data.weather.current_units.temperature_2m),
                            Style::default().fg(Color::Green),
                        ),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Humidity: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{}%", data.weather.current.relative_humidity_2m),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Pressure: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{:.1} {}", 
                                data.weather.current.pressure_msl,
                                data.weather.current_units.pressure_msl),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Wind Speed: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{:.1} {}", 
                                data.weather.current.wind_speed_10m,
                                data.weather.current_units.wind_speed_10m),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Precipitation: ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{:.1} mm", data.weather.current.precipitation),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                ];

                let weather_display = Paragraph::new(weather_text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Weather Information (Press 'i' to search again, 'q' to quit)"),
                    )
                    .wrap(Wrap { trim: true });
                f.render_widget(weather_display, main_chunks[0]);
            }
        }
        AppState::Error => {
            let error = Paragraph::new(app.error_message.as_str())
                .style(Style::default().fg(Color::Red))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Error (Press 'i' to try again)"),
                )
                .wrap(Wrap { trim: true });
            f.render_widget(error, main_chunks[0]);
        }
    }

    // History panel
    let history_border_style = if app.focused_pane == FocusedPane::History {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let history_items: Vec<ListItem> = app
        .search_history
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let style = if app.focused_pane == FocusedPane::History && i == app.selected_history_index {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(entry.query.as_str()).style(style)
        })
        .collect();

    let history = List::new(history_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(history_border_style)
            .title("History (Tab to switch, j/k to navigate, Enter to load)"),
    );
    f.render_widget(history, main_chunks[1]);

    let footer_text = match app.mode {
        Mode::Normal => "NORMAL: i=insert | Tab=switch panes | j/k=navigate history | Enter=search/load | ESC=quit",
        Mode::Insert => "INSERT: Type to search | Up/Down=select | Tab=accept/switch | ESC=normal mode",
    };
    
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

async fn fetch_weather(city: &str) -> Result<WeatherData, String> {
    let geocoding_url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        urlencoding::encode(city)
    );

    let geo_response = reqwest::get(&geocoding_url)
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "Connection timeout. Check your internet connection.".to_string()
            } else if e.is_connect() {
                "Cannot connect to weather service. Check your internet connection.".to_string()
            } else {
                format!("Network error: {}", e)
            }
        })?;

    let geo_data: GeocodingResponse = geo_response
        .json()
        .await
        .map_err(|_| "Failed to parse location data from weather service.".to_string())?;

    let location = geo_data
        .results
        .and_then(|mut r| r.pop())
        .ok_or_else(|| format!("'{}' not found. Try a different city name.", city))?;

    let weather_url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,pressure_msl&temperature_unit=celsius&wind_speed_unit=kmh&precipitation_unit=mm&timezone=auto",
        location.latitude, location.longitude
    );

    let weather_response = reqwest::get(&weather_url)
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "Connection timeout while fetching weather data.".to_string()
            } else if e.is_connect() {
                "Cannot connect to weather service.".to_string()
            } else {
                format!("Network error: {}", e)
            }
        })?;

    let weather: WeatherResponse = weather_response
        .json()
        .await
        .map_err(|_| "Failed to parse weather data from service.".to_string())?;

    Ok(WeatherData { location, weather })
}

async fn fetch_autocomplete(query: &str) -> Result<Vec<GeoLocation>, Box<dyn Error>> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        urlencoding::encode(query)
    );

    let response = reqwest::get(&url).await?;
    let data: GeocodingResponse = response.json().await?;

    Ok(data.results.unwrap_or_default())
}

fn weather_code_to_description(code: u32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
        51 | 53 | 55 => "Drizzle",
        61 | 63 | 65 => "Rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80 | 81 | 82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

