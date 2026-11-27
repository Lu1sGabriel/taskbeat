//! Ligação mínima (FFI) com as bibliotecas de texto do sistema: fontconfig
//! (qual fonte usar e as de reserva), FreeType (desenho dos glifos) e
//! HarfBuzz (posição dos glifos: kerning, ligaduras, escritas complexas).
//! As três vêm em qualquer desktop Linux (GTK e Qt dependem delas).
//!
//! Só o que o `text.rs` usa. As estruturas seguem os cabeçalhos públicos
//! (estáveis há muitos anos); campos depois dos usados ficam de fora porque
//! só lemos por ponteiro.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_long, c_short, c_uchar, c_uint, c_ulong, c_ushort, c_void};

// ---------------------------------------------------------------- FreeType

pub type FT_Error = c_int;
pub type FT_Library = *mut c_void;
pub type FT_Pos = c_long;
pub type FT_Fixed = c_long;
pub type FT_Face = *mut FT_FaceRec;

#[repr(C)]
pub struct FT_Generic {
    pub data: *mut c_void,
    pub finalizer: *mut c_void,
}

#[repr(C)]
pub struct FT_BBox {
    pub x_min: FT_Pos,
    pub y_min: FT_Pos,
    pub x_max: FT_Pos,
    pub y_max: FT_Pos,
}

#[repr(C)]
pub struct FT_Vector {
    pub x: FT_Pos,
    pub y: FT_Pos,
}

#[repr(C)]
pub struct FT_Bitmap {
    pub rows: c_uint,
    pub width: c_uint,
    pub pitch: c_int,
    pub buffer: *mut c_uchar,
    pub num_grays: c_ushort,
    pub pixel_mode: c_uchar,
    pub palette_mode: c_uchar,
    pub palette: *mut c_void,
}

#[repr(C)]
pub struct FT_Glyph_Metrics {
    pub width: FT_Pos,
    pub height: FT_Pos,
    pub hori_bearing_x: FT_Pos,
    pub hori_bearing_y: FT_Pos,
    pub hori_advance: FT_Pos,
    pub vert_bearing_x: FT_Pos,
    pub vert_bearing_y: FT_Pos,
    pub vert_advance: FT_Pos,
}

#[repr(C)]
pub struct FT_GlyphSlotRec {
    pub library: FT_Library,
    pub face: FT_Face,
    pub next: *mut FT_GlyphSlotRec,
    pub glyph_index: c_uint,
    pub generic: FT_Generic,
    pub metrics: FT_Glyph_Metrics,
    pub linear_hori_advance: FT_Fixed,
    pub linear_vert_advance: FT_Fixed,
    pub advance: FT_Vector,
    pub format: c_uint,
    pub bitmap: FT_Bitmap,
    pub bitmap_left: c_int,
    pub bitmap_top: c_int,
}

#[repr(C)]
pub struct FT_Size_Metrics {
    pub x_ppem: c_ushort,
    pub y_ppem: c_ushort,
    pub x_scale: FT_Fixed,
    pub y_scale: FT_Fixed,
    pub ascender: FT_Pos,
    pub descender: FT_Pos,
    pub height: FT_Pos,
    pub max_advance: FT_Pos,
}

#[repr(C)]
pub struct FT_SizeRec {
    pub face: FT_Face,
    pub generic: FT_Generic,
    pub metrics: FT_Size_Metrics,
    pub internal: *mut c_void,
}

#[repr(C)]
pub struct FT_FaceRec {
    pub num_faces: c_long,
    pub face_index: c_long,
    pub face_flags: c_long,
    pub style_flags: c_long,
    pub num_glyphs: c_long,
    pub family_name: *mut c_char,
    pub style_name: *mut c_char,
    pub num_fixed_sizes: c_int,
    pub available_sizes: *mut c_void,
    pub num_charmaps: c_int,
    pub charmaps: *mut c_void,
    pub generic: FT_Generic,
    pub bbox: FT_BBox,
    pub units_per_em: c_ushort,
    pub ascender: c_short,
    pub descender: c_short,
    pub height: c_short,
    pub max_advance_width: c_short,
    pub max_advance_height: c_short,
    pub underline_position: c_short,
    pub underline_thickness: c_short,
    pub glyph: *mut FT_GlyphSlotRec,
    pub size: *mut FT_SizeRec,
    pub charmap: *mut c_void,
}

pub const FT_LOAD_NO_HINTING: i32 = 1 << 1;
pub const FT_LOAD_RENDER: i32 = 1 << 2;
pub const FT_LOAD_NO_BITMAP: i32 = 1 << 3;
pub const FT_LOAD_TARGET_LIGHT: i32 = 1 << 16;
pub const FT_PIXEL_MODE_GRAY: c_uchar = 2;

#[link(name = "freetype")]
unsafe extern "C" {
    pub fn FT_Init_FreeType(lib: *mut FT_Library) -> FT_Error;
    pub fn FT_Done_FreeType(lib: FT_Library) -> FT_Error;
    pub fn FT_New_Face(lib: FT_Library, path: *const c_char, index: c_long, face: *mut FT_Face) -> FT_Error;
    pub fn FT_Done_Face(face: FT_Face) -> FT_Error;
    pub fn FT_Set_Char_Size(face: FT_Face, w: c_long, h: c_long, hres: c_uint, vres: c_uint) -> FT_Error;
    pub fn FT_Set_Transform(face: FT_Face, matrix: *mut c_void, delta: *mut FT_Vector);
    pub fn FT_Load_Glyph(face: FT_Face, glyph: c_uint, flags: i32) -> FT_Error;
    pub fn FT_Get_Char_Index(face: FT_Face, code: c_ulong) -> c_uint;
}

// ---------------------------------------------------------------- fontconfig

pub type FcBool = c_int;
pub enum FcConfig {}
pub enum FcPattern {}
pub enum FcCharSet {}

#[repr(C)]
pub struct FcFontSet {
    pub nfont: c_int,
    pub sfont: c_int,
    pub fonts: *mut *mut FcPattern,
}

pub const FC_RESULT_MATCH: c_int = 0;
pub const FC_MATCH_PATTERN: c_int = 0;
pub const FC_WEIGHT_REGULAR: c_int = 80;
pub const FC_WEIGHT_MEDIUM: c_int = 100;
pub const FC_WEIGHT_DEMIBOLD: c_int = 180;
pub const FC_WEIGHT_BOLD: c_int = 200;

#[link(name = "fontconfig")]
unsafe extern "C" {
    pub fn FcInit() -> FcBool;
    pub fn FcNameParse(name: *const c_uchar) -> *mut FcPattern;
    pub fn FcPatternDestroy(p: *mut FcPattern);
    pub fn FcPatternAddInteger(p: *mut FcPattern, object: *const c_char, i: c_int) -> FcBool;
    pub fn FcConfigSubstitute(config: *mut FcConfig, p: *mut FcPattern, kind: c_int) -> FcBool;
    pub fn FcDefaultSubstitute(p: *mut FcPattern);
    pub fn FcFontSort(config: *mut FcConfig, p: *mut FcPattern, trim: FcBool, csp: *mut *mut FcCharSet, result: *mut c_int) -> *mut FcFontSet;
    pub fn FcFontSetDestroy(s: *mut FcFontSet);
    pub fn FcPatternGetString(p: *const FcPattern, object: *const c_char, n: c_int, s: *mut *mut c_uchar) -> c_int;
    pub fn FcPatternGetInteger(p: *const FcPattern, object: *const c_char, n: c_int, i: *mut c_int) -> c_int;
    pub fn FcPatternGetBool(p: *const FcPattern, object: *const c_char, n: c_int, b: *mut FcBool) -> c_int;
    pub fn FcPatternGetCharSet(p: *const FcPattern, object: *const c_char, n: c_int, c: *mut *mut FcCharSet) -> c_int;
    pub fn FcCharSetCopy(c: *mut FcCharSet) -> *mut FcCharSet;
    pub fn FcCharSetHasChar(c: *const FcCharSet, ucs4: u32) -> FcBool;
}

// ---------------------------------------------------------------- HarfBuzz

pub enum hb_font_t {}
pub enum hb_buffer_t {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct hb_glyph_info_t {
    pub codepoint: u32,
    pub mask: u32,
    pub cluster: u32,
    pub var1: u32,
    pub var2: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct hb_glyph_position_t {
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub var: u32,
}

#[link(name = "harfbuzz")]
unsafe extern "C" {
    pub fn hb_ft_font_create_referenced(face: FT_Face) -> *mut hb_font_t;
    pub fn hb_ft_font_changed(font: *mut hb_font_t);
    pub fn hb_ft_font_set_load_flags(font: *mut hb_font_t, flags: c_int);
    pub fn hb_font_destroy(font: *mut hb_font_t);
    pub fn hb_buffer_create() -> *mut hb_buffer_t;
    pub fn hb_buffer_destroy(buf: *mut hb_buffer_t);
    pub fn hb_buffer_clear_contents(buf: *mut hb_buffer_t);
    pub fn hb_buffer_add_utf8(buf: *mut hb_buffer_t, text: *const c_char, text_length: c_int, item_offset: c_uint, item_length: c_int);
    pub fn hb_buffer_guess_segment_properties(buf: *mut hb_buffer_t);
    pub fn hb_shape(font: *mut hb_font_t, buf: *mut hb_buffer_t, features: *const c_void, num_features: c_uint);
    pub fn hb_buffer_get_glyph_infos(buf: *mut hb_buffer_t, length: *mut c_uint) -> *mut hb_glyph_info_t;
    pub fn hb_buffer_get_glyph_positions(buf: *mut hb_buffer_t, length: *mut c_uint) -> *mut hb_glyph_position_t;
}
