//! A small RFB (VNC) client over a WebSocket, for a bot's screen.
//!
//! Hermes streams each bot's Xvnc desktop as raw RFB 3.8 bytes over
//! `/api/display/ws` with security type None. We ask for 32-bit true colour
//! laid out as `0xffRRGGBB` (BGRA bytes in memory) and accept ZRLE, CopyRect and Raw,
//! plus the DesktopSize pseudo-encoding. Input is only forwarded by the
//! server while this viewer holds the human control lease.

use std::sync::{Arc, Mutex};

use flate2::{Decompress, FlushDecompress, Status};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// The screen's pixels, `0xffRRGGBB` per pixel, row-major.
#[derive(Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Frame {
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixels = vec![0xff00_0000; (width as usize) * (height as usize)];
    }

    fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        for row in y..(y + h).min(self.height) {
            let start = (row * self.width + x) as usize;
            let end = start + (w.min(self.width.saturating_sub(x))) as usize;
            self.pixels[start..end].fill(color);
        }
    }

    fn put(&mut self, x: u32, y: u32, color: u32) {
        if x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = color;
        }
    }

    fn copy_rect(&mut self, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32) {
        let (fw, fh) = (self.width, self.height);
        if sx + w > fw || dx + w > fw || sy + h > fh || dy + h > fh {
            return;
        }
        // Rows in an order that never reads a row already overwritten.
        let rows: Box<dyn Iterator<Item = u32>> = if dy > sy { Box::new((0..h).rev()) } else { Box::new(0..h) };
        for r in rows {
            let src = ((sy + r) * fw + sx) as usize;
            let dst = ((dy + r) * fw + dx) as usize;
            self.pixels.copy_within(src..src + w as usize, dst);
        }
    }
}

pub type SharedFrame = Arc<Mutex<Frame>>;

/// Input for the server; dropped by it unless we hold the control lease.
#[derive(Debug, Clone)]
pub enum Input {
    Pointer { mask: u8, x: u16, y: u16 },
    Key { down: bool, keysym: u32 },
}

#[derive(Debug, Clone)]
pub enum Event {
    Ready { width: u32, height: u32 },
    /// Pixels changed; repaint.
    Updated,
}

/// How a session ended. `code` is the WebSocket close code when there was one:
/// 4000 control taken by another viewer, 4001 screen not running, 4401 ticket
/// rejected.
#[derive(Debug, Clone)]
pub struct Ended {
    pub code: Option<u16>,
    pub reason: String,
}

fn ended(reason: impl Into<String>) -> Ended {
    Ended { code: None, reason: reason.into() }
}

/// Buffers WebSocket binary frames into a byte stream for the RFB parser.
struct Reader<S> {
    stream: S,
    buf: Vec<u8>,
    pos: usize,
}

impl<S> Reader<S>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    async fn fill(&mut self, n: usize) -> Result<(), Ended> {
        while self.buf.len() - self.pos < n {
            match self.stream.next().await {
                Some(Ok(Message::Binary(data))) => {
                    if self.pos > 0 {
                        self.buf.drain(..self.pos);
                        self.pos = 0;
                    }
                    self.buf.extend_from_slice(&data);
                }
                Some(Ok(Message::Close(frame))) => {
                    return Err(match frame {
                        Some(f) => Ended { code: Some(u16::from(f.code)), reason: f.reason.to_string() },
                        None => ended("The screen connection closed."),
                    });
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(ended(format!("Screen connection lost: {e}"))),
                None => return Err(ended("Screen connection lost.")),
            }
        }
        Ok(())
    }

    async fn bytes(&mut self, n: usize) -> Result<&[u8], Ended> {
        self.fill(n).await?;
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    async fn u8(&mut self) -> Result<u8, Ended> {
        Ok(self.bytes(1).await?[0])
    }

    async fn u16(&mut self) -> Result<u16, Ended> {
        let b = self.bytes(2).await?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    async fn u32(&mut self) -> Result<u32, Ended> {
        let b = self.bytes(4).await?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    async fn string(&mut self) -> Result<String, Ended> {
        let len = self.u32().await? as usize;
        if len > 1 << 20 {
            return Err(ended("The screen server sent an oversized message."));
        }
        Ok(String::from_utf8_lossy(self.bytes(len).await?).into_owned())
    }
}

/// 32 bpp, depth 24, little-endian true colour with red at bit 16: exactly
/// `QImage::Format_RGB32` once the top byte is set.
const PIXEL_FORMAT: [u8; 16] = [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];

const ENC_RAW: i32 = 0;
const ENC_COPY_RECT: i32 = 1;
const ENC_ZRLE: i32 = 16;
const ENC_DESKTOP_SIZE: i32 = -223;

fn set_encodings() -> Vec<u8> {
    let encodings = [ENC_ZRLE, ENC_COPY_RECT, ENC_RAW, ENC_DESKTOP_SIZE];
    let mut msg = vec![2, 0];
    msg.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for e in encodings {
        msg.extend_from_slice(&e.to_be_bytes());
    }
    msg
}

fn update_request(incremental: bool, width: u32, height: u32) -> Vec<u8> {
    let mut msg = vec![3, incremental as u8, 0, 0, 0, 0];
    msg.extend_from_slice(&(width.min(u16::MAX as u32) as u16).to_be_bytes());
    msg.extend_from_slice(&(height.min(u16::MAX as u32) as u16).to_be_bytes());
    msg
}

pub fn encode_input(input: &Input) -> Vec<u8> {
    match input {
        Input::Pointer { mask, x, y } => {
            let mut m = vec![5, *mask];
            m.extend_from_slice(&x.to_be_bytes());
            m.extend_from_slice(&y.to_be_bytes());
            m
        }
        Input::Key { down, keysym } => {
            let mut m = vec![4, *down as u8, 0, 0];
            m.extend_from_slice(&keysym.to_be_bytes());
            m
        }
    }
}

/// Runs one viewing session until the server or the network ends it.
pub async fn run(
    url: &str,
    frame: SharedFrame,
    mut input: mpsc::UnboundedReceiver<Input>,
    on_event: impl Fn(Event) + Send + Sync + 'static,
) -> Ended {
    let socket = match tokio_tungstenite::connect_async(url).await {
        Ok((socket, _)) => socket,
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => {
            return ended(format!("The server refused the screen connection (HTTP {}).", r.status().as_u16()));
        }
        Err(e) => return ended(format!("Couldn't connect to the screen: {e}")),
    };
    let (mut sink, stream) = socket.split();
    let mut reader = Reader { stream, buf: Vec::new(), pos: 0 };

    // ---- handshake ----
    let handshake = async {
        let version = reader.bytes(12).await?.to_vec();
        if !version.starts_with(b"RFB 003.") {
            return Err(ended("The screen server doesn't speak VNC."));
        }
        let minor: u32 = String::from_utf8_lossy(&version[8..11]).parse().unwrap_or(3);
        let minor = if minor >= 8 { 8 } else if minor == 7 { 7 } else { 3 };
        let ours = format!("RFB 003.00{minor}\n");
        Ok::<_, Ended>((minor, ours))
    };
    let (minor, ours) = match handshake.await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if sink.send(Message::binary(ours.into_bytes())).await.is_err() {
        return ended("Screen connection lost.");
    }

    let security = async {
        if minor >= 7 {
            let count = reader.u8().await?;
            if count == 0 {
                return Err(ended(format!("The screen refused the connection: {}", reader.string().await?)));
            }
            let types = reader.bytes(count as usize).await?.to_vec();
            if !types.contains(&1) {
                return Err(ended("The screen asks for a VNC password, which Hermes doesn't use."));
            }
            Ok(true)
        } else {
            match reader.u32().await? {
                1 => Ok(false),
                0 => Err(ended(format!("The screen refused the connection: {}", reader.string().await?))),
                _ => Err(ended("The screen asks for a VNC password, which Hermes doesn't use.")),
            }
        }
    };
    let choose = match security.await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if choose && sink.send(Message::binary(vec![1u8])).await.is_err() {
        return ended("Screen connection lost.");
    }
    if minor >= 8 {
        match reader.u32().await {
            Ok(0) => {}
            Ok(_) => {
                return ended(format!(
                    "The screen refused the connection: {}",
                    reader.string().await.unwrap_or_default()
                ));
            }
            Err(e) => return e,
        }
    }
    // ClientInit: shared, so the official app and others can watch too.
    if sink.send(Message::binary(vec![1u8])).await.is_err() {
        return ended("Screen connection lost.");
    }
    let init = async {
        let width = reader.u16().await? as u32;
        let height = reader.u16().await? as u32;
        reader.bytes(16).await?; // the server's pixel format; we set our own
        reader.string().await?; // the desktop's name; we don't show it
        Ok::<_, Ended>((width, height))
    };
    let (width, height) = match init.await {
        Ok(v) => v,
        Err(e) => return e,
    };
    frame.lock().unwrap().resize(width, height);
    on_event(Event::Ready { width, height });

    // ---- writer: our requests and the user's input ----
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let mut set_format = vec![0u8, 0, 0, 0];
    set_format.extend_from_slice(&PIXEL_FORMAT);
    let _ = out_tx.send(set_format);
    let _ = out_tx.send(set_encodings());
    let _ = out_tx.send(update_request(false, width, height));
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = out_rx.recv() => match msg {
                    Some(bytes) => if sink.send(Message::binary(bytes)).await.is_err() { break },
                    None => break,
                },
                user = input.recv() => match user {
                    Some(event) => if sink.send(Message::binary(encode_input(&event))).await.is_err() { break },
                    // The viewer went away: close cleanly, which also hands
                    // back the control lease if we held it.
                    None => break,
                },
            }
        }
        let _ = sink.send(Message::Close(None)).await;
    });

    // ---- reader: server messages ----
    let mut inflater = Decompress::new(true);
    let result: Result<(), Ended> = async {
        loop {
            match reader.u8().await? {
                0 => {
                    reader.u8().await?; // padding
                    let rects = reader.u16().await?;
                    let mut resized = None;
                    for _ in 0..rects {
                        let x = reader.u16().await? as u32;
                        let y = reader.u16().await? as u32;
                        let w = reader.u16().await? as u32;
                        let h = reader.u16().await? as u32;
                        let encoding = reader.u32().await? as i32;
                        match encoding {
                            ENC_RAW => {
                                let data = reader.bytes((w * h * 4) as usize).await?;
                                let mut f = frame.lock().unwrap();
                                for (i, px) in data.chunks_exact(4).enumerate() {
                                    let color = 0xff00_0000 | u32::from_le_bytes([px[0], px[1], px[2], 0]);
                                    f.put(x + (i as u32 % w.max(1)), y + (i as u32 / w.max(1)), color);
                                }
                            }
                            ENC_COPY_RECT => {
                                let sx = reader.u16().await? as u32;
                                let sy = reader.u16().await? as u32;
                                frame.lock().unwrap().copy_rect(sx, sy, x, y, w, h);
                            }
                            ENC_ZRLE => {
                                let len = reader.u32().await? as usize;
                                let compressed = reader.bytes(len).await?.to_vec();
                                let data = inflate(&mut inflater, &compressed)?;
                                decode_zrle(&data, x, y, w, h, &mut frame.lock().unwrap())?;
                            }
                            ENC_DESKTOP_SIZE => resized = Some((w, h)),
                            other => return Err(ended(format!("The screen sent an unsupported encoding ({other})."))),
                        }
                    }
                    let (fw, fh) = match resized {
                        Some((w, h)) => {
                            frame.lock().unwrap().resize(w, h);
                            on_event(Event::Ready { width: w, height: h });
                            let _ = out_tx.send(update_request(false, w, h));
                            continue;
                        }
                        None => {
                            let f = frame.lock().unwrap();
                            (f.width, f.height)
                        }
                    };
                    on_event(Event::Updated);
                    let _ = out_tx.send(update_request(true, fw, fh));
                }
                1 => {
                    // SetColourMapEntries: unused in true colour; skip it.
                    reader.bytes(3).await?;
                    let count = reader.u16().await? as usize;
                    reader.bytes(count * 6).await?;
                }
                2 => {} // Bell
                3 => {
                    reader.bytes(3).await?;
                    reader.string().await?; // ServerCutText (Hermes disables it)
                }
                other => return Err(ended(format!("The screen sent an unknown message ({other})."))),
            }
        }
    }
    .await;
    writer.abort();
    result.err().unwrap_or_else(|| ended("Screen connection ended."))
}

/// Inflates one ZRLE rectangle; the zlib stream continues across rectangles.
fn inflate(inflater: &mut Decompress, input: &[u8]) -> Result<Vec<u8>, Ended> {
    inflate_step(inflater, input, 64 * 1024)
}

/// Keeps calling zlib until it has consumed the whole rectangle AND stopped
/// producing output: when the buffer fills exactly as the last input byte is
/// read, the rest of the rectangle is still inside zlib, and stopping on
/// "input used up" alone truncates it.
fn inflate_step(inflater: &mut Decompress, input: &[u8], step: usize) -> Result<Vec<u8>, Ended> {
    let mut out: Vec<u8> = Vec::with_capacity(step.max(input.len() * 4));
    let start_in = inflater.total_in();
    loop {
        if out.capacity() == out.len() {
            out.reserve_exact(step.max(out.len() / 2));
        }
        let consumed = (inflater.total_in() - start_in) as usize;
        let (before_in, before_out) = (inflater.total_in(), inflater.total_out());
        let status = inflater
            .decompress_vec(&input[consumed.min(input.len())..], &mut out, FlushDecompress::Sync)
            .map_err(|e| ended(format!("Bad screen data: {e}")))?;
        if matches!(status, Status::StreamEnd) {
            break;
        }
        let progressed = inflater.total_in() > before_in || inflater.total_out() > before_out;
        if !progressed {
            if out.capacity() > out.len() {
                // Room to write and nothing came out: everything is drained.
                break;
            }
            // Buffer exactly full: grow it and ask again.
        }
    }
    let consumed = (inflater.total_in() - start_in) as usize;
    if consumed < input.len() {
        return Err(ended("Bad screen data: compressed data left over."));
    }
    Ok(out)
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Ended> {
        if self.pos + n > self.data.len() {
            return Err(ended(format!(
                "Truncated screen data (needed {n} bytes at {} of {}).",
                self.pos,
                self.data.len()
            )));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, Ended> {
        Ok(self.take(1)?[0])
    }
    /// A ZRLE "CPIXEL": the three low bytes of our 32-bit pixel.
    fn cpixel(&mut self) -> Result<u32, Ended> {
        let b = self.take(3)?;
        Ok(0xff00_0000 | u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }
    fn run_length(&mut self) -> Result<u32, Ended> {
        let mut len = 1u32;
        loop {
            let b = self.u8()?;
            len += b as u32;
            if b != 255 {
                return Ok(len);
            }
        }
    }
}

/// Decodes a ZRLE rectangle (64×64 tiles) into `frame`.
pub fn decode_zrle(data: &[u8], x: u32, y: u32, w: u32, h: u32, frame: &mut Frame) -> Result<(), Ended> {
    let mut c = Cursor { data, pos: 0 };
    for ty in (0..h).step_by(64) {
        for tx in (0..w).step_by(64) {
            let (tw, th) = ((w - tx).min(64), (h - ty).min(64));
            let (ox, oy) = (x + tx, y + ty);
            let sub = c.u8()?;
            let tile = |e: Ended| Ended {
                code: e.code,
                reason: format!("{} [ZRLE {w}x{h} rect, tile {tx},{ty} {tw}x{th}, subencoding {sub}]", e.reason),
            };
            decode_tile(&mut c, sub, ox, oy, tw, th, frame).map_err(tile)?;
        }
    }
    Ok(())
}

fn decode_tile(c: &mut Cursor, sub: u8, ox: u32, oy: u32, tw: u32, th: u32, frame: &mut Frame) -> Result<(), Ended> {
    {
        {
            match sub {
                0 => {
                    for j in 0..th {
                        for i in 0..tw {
                            let color = c.cpixel()?;
                            frame.put(ox + i, oy + j, color);
                        }
                    }
                }
                1 => {
                    let color = c.cpixel()?;
                    frame.fill(ox, oy, tw, th, color);
                }
                2..=16 => {
                    let palette: Vec<u32> = (0..sub).map(|_| c.cpixel()).collect::<Result<_, _>>()?;
                    let bits = if sub == 2 { 1 } else if sub <= 4 { 2 } else { 4 };
                    let row_bytes = (tw * bits).div_ceil(8) as usize;
                    for j in 0..th {
                        let row = c.take(row_bytes)?.to_vec();
                        for i in 0..tw {
                            let bit = (i * bits) as usize;
                            let byte = row[bit / 8];
                            let shift = 8 - bits as usize - (bit % 8);
                            let index = ((byte >> shift) & ((1 << bits) - 1) as u8) as usize;
                            frame.put(ox + i, oy + j, *palette.get(index).unwrap_or(&0xff00_0000));
                        }
                    }
                }
                128 | 130..=255 => {
                    let palette: Vec<u32> = if sub == 128 {
                        Vec::new()
                    } else {
                        (0..sub - 128).map(|_| c.cpixel()).collect::<Result<_, _>>()?
                    };
                    let total = tw * th;
                    let mut filled = 0u32;
                    while filled < total {
                        let (color, run) = if sub == 128 {
                            let color = c.cpixel()?;
                            (color, c.run_length()?)
                        } else {
                            let index = c.u8()?;
                            let color = *palette.get((index & 127) as usize).unwrap_or(&0xff00_0000);
                            (color, if index & 128 != 0 { c.run_length()? } else { 1 })
                        };
                        for _ in 0..run.min(total - filled) {
                            frame.put(ox + filled % tw, oy + filled / tw, color);
                            filled += 1;
                        }
                    }
                }
                _ => return Err(ended(format!("Bad screen tile ({sub})."))),
            }
        }
    }
    Ok(())
}

/// X11 keysym for a key as the windowing layer names it (`"escape"`,
/// `"f5"`, `"a"`) and the text it types, when it types any.
pub fn keysym(key: &str, text: Option<&str>) -> Option<u32> {
    let special = match key {
        "escape" => 0xff1b,
        "tab" => 0xff09,
        "backspace" => 0xff08,
        "enter" => 0xff0d,
        "insert" => 0xff63,
        "delete" => 0xffff,
        "pause" => 0xff13,
        "printscreen" => 0xff61,
        "home" => 0xff50,
        "end" => 0xff57,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "shift" => 0xffe1,
        "control" => 0xffe3,
        "platform" => 0xffeb,
        "alt" => 0xffe9,
        "capslock" => 0xffe5,
        "menu" => 0xff67,
        "space" => 0x20,
        k if k.len() > 1 && k.starts_with('f') => match k[1..].parse::<u32>() {
            Ok(n @ 1..=35) => 0xffbe + n - 1,
            _ => 0,
        },
        _ => 0,
    };
    if special != 0 {
        return Some(special);
    }
    let printable = |s: &str| {
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if (ch as u32) >= 0x20 && ch as u32 != 0x7f => Some(ch as u32),
            _ => None,
        }
    };
    // What the key types (shifted, layout-aware); with Ctrl held there is no
    // text, so the key itself.
    let cp = text.and_then(printable).or_else(|| printable(key))?;
    Some(if cp < 0x100 { cp } else { 0x0100_0000 + cp })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> Frame {
        let mut f = Frame::default();
        f.resize(w, h);
        f
    }

    #[test]
    fn decodes_solid_and_raw_zrle_tiles() {
        let mut f = frame(70, 2);
        // Two tiles across (64 + 6 wide): a solid red tile, then a raw tile.
        let mut data = vec![1, 0x00, 0x00, 0xff];
        data.push(0);
        for i in 0..12u8 {
            data.extend_from_slice(&[i, 0, 0]);
        }
        decode_zrle(&data, 0, 0, 70, 2, &mut f).unwrap();
        assert_eq!(f.pixels[0], 0xffff_0000);
        assert_eq!(f.pixels[63], 0xffff_0000);
        assert_eq!(f.pixels[64], 0xff00_0000);
        assert_eq!(f.pixels[65], 0xff00_0001);
        assert_eq!(f.pixels[70 + 69], 0xff00_000b);
    }

    #[test]
    fn decodes_packed_palette_and_rle_tiles() {
        let mut f = frame(4, 1);
        // Two-colour palette, 1 bit per pixel: 1,0,1,1.
        let data = [2, 1, 0, 0, 2, 0, 0, 0b1011_0000];
        decode_zrle(&data, 0, 0, 4, 1, &mut f).unwrap();
        assert_eq!(f.pixels, vec![0xff00_0002, 0xff00_0001, 0xff00_0002, 0xff00_0002]);

        let mut f = frame(4, 1);
        // Plain RLE: colour 5 for 3 pixels, then colour 6 for 1.
        let data = [128, 5, 0, 0, 2, 6, 0, 0, 0];
        decode_zrle(&data, 0, 0, 4, 1, &mut f).unwrap();
        assert_eq!(f.pixels, vec![0xff00_0005, 0xff00_0005, 0xff00_0005, 0xff00_0006]);

        let mut f = frame(4, 1);
        // Palette RLE: index 1 run of 3 (0x81, len byte 2), then index 0 once.
        let data = [130, 7, 0, 0, 8, 0, 0, 0x81, 2, 0x00];
        decode_zrle(&data, 0, 0, 4, 1, &mut f).unwrap();
        assert_eq!(f.pixels, vec![0xff00_0008, 0xff00_0008, 0xff00_0008, 0xff00_0007]);
    }

    /// ZRLE shares one zlib stream across rectangles, flushed per rectangle.
    #[test]
    fn inflates_a_stream_shared_across_rects() {
        use flate2::{Compress, Compression, FlushCompress};
        let mut deflater = Compress::new(Compression::default(), true);
        let mut chunk = |input: &[u8]| {
            let mut out = Vec::with_capacity(1024);
            deflater.compress_vec(input, &mut out, FlushCompress::Sync).unwrap();
            out
        };
        let first = chunk(&[1, 0x11, 0x22, 0x33]);
        let second = chunk(&[1, 0x44, 0x55, 0x66]);
        let mut inflater = Decompress::new(true);
        let mut f = frame(2, 2);
        decode_zrle(&inflate(&mut inflater, &first).unwrap(), 0, 0, 2, 1, &mut f).unwrap();
        decode_zrle(&inflate(&mut inflater, &second).unwrap(), 0, 1, 2, 1, &mut f).unwrap();
        assert_eq!(f.pixels, vec![0xff33_2211, 0xff33_2211, 0xff66_5544, 0xff66_5544]);
    }

    /// A full-screen raw ZRLE update of a gradient: far more output than input.
    #[test]
    fn decodes_a_full_screen_compressed_rect() {
        use flate2::{Compress, Compression, FlushCompress};
        let (w, h) = (1440u32, 900u32);
        let mut raw = Vec::new();
        for ty in (0..h).step_by(64) {
            for tx in (0..w).step_by(64) {
                let (tw, th) = ((w - tx).min(64), (h - ty).min(64));
                raw.push(0u8);
                for j in 0..th {
                    for i in 0..tw {
                        raw.extend_from_slice(&[((tx + i) / 6) as u8, ((ty + j) / 4) as u8, 0x20]);
                    }
                }
            }
        }
        let mut deflater = Compress::new(Compression::default(), true);
        let mut compressed = Vec::with_capacity(raw.len() + 1024);
        deflater.compress_vec(&raw, &mut compressed, FlushCompress::Sync).unwrap();
        let mut inflater = Decompress::new(true);
        let data = inflate(&mut inflater, &compressed).unwrap();
        assert_eq!(data.len(), raw.len());
        let mut f = frame(w, h);
        decode_zrle(&data, 0, 0, w, h, &mut f).unwrap();
        // CPIXEL bytes are blue, green, red.
        assert_eq!(f.pixels[(899 * 1440 + 1439) as usize], 0xff00_0000 | (0x20 << 16) | (224 << 8) | 239);
    }

    #[test]
    fn inflate_survives_any_buffer_boundary() {
        use flate2::{Compress, Compression, FlushCompress};
        let raw: Vec<u8> = (0..400_000u32).map(|i| ((i / 3) % 251) as u8 ^ ((i / 997) as u8)).collect();
        let mut deflater = Compress::new(Compression::default(), true);
        let mut compressed = Vec::with_capacity(raw.len() + 1024);
        deflater.compress_vec(&raw, &mut compressed, FlushCompress::Sync).unwrap();
        for step in [1usize, 3, 7, 64, 1000, 4096, 65536] {
            let mut inflater = Decompress::new(true);
            let out = inflate_step(&mut inflater, &compressed, step).unwrap();
            assert_eq!(out.len(), raw.len(), "step {step}");
        }
    }

    #[test]
    fn copies_overlapping_rects() {
        let mut f = frame(3, 3);
        for (i, p) in f.pixels.iter_mut().enumerate() {
            *p = i as u32;
        }
        f.copy_rect(0, 0, 0, 1, 3, 2);
        assert_eq!(f.pixels, vec![0, 1, 2, 0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn maps_keys_to_keysyms() {
        assert_eq!(keysym("a", Some("a")), Some(0x61));
        assert_eq!(keysym("a", Some("A")), Some(0x41));
        assert_eq!(keysym("enter", None), Some(0xff0d));
        assert_eq!(keysym("f1", None), Some(0xffbe));
        assert_eq!(keysym("c", None), Some(0x63));
        assert_eq!(keysym("e", Some("é")), Some(0xe9));
        assert_eq!(keysym("e", Some("€")), Some(0x0100_20ac));
    }

    /// A fake Xvnc: RFB 3.8 handshake, one Raw update, then close 4001.
    #[tokio::test]
    async fn speaks_rfb_over_a_websocket() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.send(Message::binary(b"RFB 003.008\n".to_vec())).await.unwrap();
            let Some(Ok(Message::Binary(v))) = ws.next().await else { panic!() };
            assert_eq!(&v[..], b"RFB 003.008\n");
            ws.send(Message::binary(vec![1u8, 1])).await.unwrap();
            let Some(Ok(Message::Binary(choice))) = ws.next().await else { panic!() };
            assert_eq!(&choice[..], &[1]);
            ws.send(Message::binary(0u32.to_be_bytes().to_vec())).await.unwrap();
            let Some(Ok(Message::Binary(_shared))) = ws.next().await else { panic!() };
            let mut init = vec![0, 2, 0, 1];
            init.extend_from_slice(&[0; 16]);
            init.extend_from_slice(&4u32.to_be_bytes());
            init.extend_from_slice(b"test");
            ws.send(Message::binary(init)).await.unwrap();
            // SetPixelFormat, SetEncodings, FramebufferUpdateRequest.
            for expected in [0u8, 2, 3] {
                let Some(Ok(Message::Binary(m))) = ws.next().await else { panic!() };
                assert_eq!(m[0], expected);
            }
            // One Raw 2x1 update, split across frames on purpose.
            let mut update = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1];
            update.extend_from_slice(&0i32.to_be_bytes());
            update.extend_from_slice(&[0x10, 0x20, 0x30, 0, 0x01, 0x02]);
            ws.send(Message::binary(update)).await.unwrap();
            ws.send(Message::binary(vec![0x03, 0])).await.unwrap();
            // The incremental request that follows the update.
            let Some(Ok(Message::Binary(m))) = ws.next().await else { panic!() };
            assert_eq!(m[0..2], [3, 1]);
            ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: 4001.into(),
                reason: "Bot Desktop is not running".into(),
            }))
            .await
            .unwrap();
        });

        let frame: SharedFrame = Arc::default();
        let (_tx, rx) = mpsc::unbounded_channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let end = run(&format!("ws://127.0.0.1:{port}/api/display/ws"), frame.clone(), rx, move |e| {
            sink.lock().unwrap().push(format!("{e:?}"))
        })
        .await;
        assert_eq!(end.code, Some(4001));
        let f = frame.lock().unwrap();
        assert_eq!((f.width, f.height), (2, 1));
        assert_eq!(f.pixels, vec![0xff30_2010, 0xff03_0201]);
        assert!(events.lock().unwrap().iter().any(|e| e.starts_with("Ready")));
    }
}

/// Watches a real bot screen (view only) with the saved sign-in and reports
/// how the session ends: `HERMES_BOT_PROFILE=default cargo test rfb_live -- --ignored --nocapture`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::backend::{gateway::Gateway, store};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    #[ignore]
    async fn rfb_live() {
        let Ok(profile) = std::env::var("HERMES_BOT_PROFILE") else { return };
        let config = store::load(&store::config_path()).unwrap();
        let server = config.server(config.active_server.as_deref().unwrap()).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();
        let observed = gateway.request("display.observe", json!({ "profile": profile })).await.unwrap();
        let path = observed.get("path").and_then(Value::as_str).unwrap_or("/api/display/ws");
        let base = server.url.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
        let mut url = url::Url::parse(&format!("{base}{path}")).unwrap();
        url.query_pairs_mut().append_pair("display_ticket", observed["ticket"].as_str().unwrap());

        let (_input, rx) = mpsc::unbounded_channel();
        let updates = Arc::new(AtomicUsize::new(0));
        let counter = updates.clone();
        let session = run(url.as_str(), Arc::default(), rx, move |e| match e {
            Event::Updated => {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            other => println!("{other:?}"),
        });
        match tokio::time::timeout(std::time::Duration::from_secs(20), session).await {
            Ok(end) => println!("ENDED after {} updates: {end:?}", updates.load(Ordering::Relaxed)),
            Err(_) => println!("STILL RUNNING after 20 s, {} updates", updates.load(Ordering::Relaxed)),
        }
    }
}
