use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const CPU_TYPE_X86: u32 = 7;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Architecture {
    I386,
    X86_64,
    Arm64,
    Other(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryKind {
    MachO(Vec<Architecture>),
    PortableExecutable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeArchitecture {
    I386,
    X86_64,
    Arm64,
    Other(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PeGraphicsApi {
    D3d8,
    D3d9,
    D3d10,
    D3d11,
    D3d12,
    OpenGl,
}

impl PeGraphicsApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::D3d8 => "d3d8",
            Self::D3d9 => "d3d9",
            Self::D3d10 => "d3d10",
            Self::D3d11 => "d3d11",
            Self::D3d12 => "d3d12",
            Self::OpenGl => "opengl",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeGraphicsEvidence {
    pub apis: BTreeSet<PeGraphicsApi>,
    pub inspected_files: Vec<String>,
    pub imported_libraries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PeInputApi {
    XInput,
    DirectInput,
    Sdl,
}

impl PeInputApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XInput => "xinput",
            Self::DirectInput => "directinput",
            Self::Sdl => "sdl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PeAudioApi {
    XAudio,
    DirectSound,
    OpenAl,
    Fmod,
    Miles,
    Sdl,
    WinMm,
}

impl PeAudioApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XAudio => "xaudio",
            Self::DirectSound => "directsound",
            Self::OpenAl => "openal",
            Self::Fmod => "fmod",
            Self::Miles => "miles",
            Self::Sdl => "sdl",
            Self::WinMm => "winmm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PePrerequisite {
    LegacyDirectX,
    VisualCpp,
    PhysX,
}

impl PePrerequisite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacyDirectX => "legacy-directx",
            Self::VisualCpp => "visual-cpp",
            Self::PhysX => "physx",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeCompatibilityEvidence {
    pub graphics: BTreeSet<PeGraphicsApi>,
    pub input: BTreeSet<PeInputApi>,
    pub audio: BTreeSet<PeAudioApi>,
    pub prerequisites: BTreeSet<PePrerequisite>,
    pub imported_libraries: Vec<String>,
    pub local_libraries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachOFileType {
    Executable,
    Dylib,
    Bundle,
    Other(u32),
}

pub fn inspect(path: &Path) -> io::Result<BinaryKind> {
    let mut file = File::open(path)?;
    let mut bytes = [0_u8; 4096];
    let count = file.read(&mut bytes)?;
    Ok(inspect_bytes(&bytes[..count]))
}

pub fn inspect_pe_architecture(path: &Path) -> io::Result<Option<PeArchitecture>> {
    let mut file = File::open(path)?;
    let mut dos_header = [0_u8; 64];
    if file.read(&mut dos_header)? != dos_header.len() || !dos_header.starts_with(b"MZ") {
        return Ok(None);
    }
    let pe_offset = u32::from_le_bytes(dos_header[0x3c..0x40].try_into().unwrap()) as u64;
    file.seek(SeekFrom::Start(pe_offset))?;
    let mut pe_header = [0_u8; 6];
    if file.read(&mut pe_header)? != pe_header.len() || &pe_header[..4] != b"PE\0\0" {
        return Ok(None);
    }
    let machine = u16::from_le_bytes(pe_header[4..6].try_into().unwrap());
    Ok(Some(match machine {
        0x014c => PeArchitecture::I386,
        0x8664 => PeArchitecture::X86_64,
        0xaa64 => PeArchitecture::Arm64,
        other => PeArchitecture::Other(other),
    }))
}

pub fn inspect_pe_imports(path: &Path) -> io::Result<Vec<String>> {
    pe_imports_from_bytes(&fs::read(path)?)
}

/// Inspects the configured PE entrypoint and locally bundled DLLs reachable
/// from its import table. The traversal is bounded so launch planning cannot
/// turn into an unbounded directory scan.
pub fn inspect_pe_graphics(path: &Path) -> io::Result<PeGraphicsEvidence> {
    const MAX_FILES: usize = 64;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let mut queue = vec![path.to_path_buf()];
    let mut visited = BTreeSet::new();
    let mut apis = BTreeSet::new();
    let mut imported_libraries = BTreeSet::new();
    let mut inspected_files = Vec::new();

    while let Some(candidate) = queue.pop() {
        if inspected_files.len() >= MAX_FILES {
            break;
        }
        let key = candidate
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if key.is_empty() || !visited.insert(key) {
            continue;
        }
        let imports = inspect_pe_imports(&candidate)?;
        inspected_files.push(
            candidate
                .strip_prefix(root)
                .unwrap_or(&candidate)
                .display()
                .to_string(),
        );
        for imported in imports {
            let normalized = imported.to_ascii_lowercase();
            imported_libraries.insert(normalized.clone());
            if let Some(api) = graphics_api_for_library(&normalized) {
                apis.insert(api);
            }
            if let Some(local) = find_case_insensitive_child(root, &imported)
                && matches!(inspect(&local), Ok(BinaryKind::PortableExecutable))
            {
                queue.push(local);
            }
        }
    }
    inspected_files.sort();
    Ok(PeGraphicsEvidence {
        apis,
        inspected_files,
        imported_libraries: imported_libraries.into_iter().collect(),
    })
}

pub fn inspect_pe_compatibility(path: &Path) -> io::Result<PeCompatibilityEvidence> {
    const MAX_LOCAL_FILES: usize = 4096;
    let graphics_evidence = inspect_pe_graphics(path)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let mut local_libraries = fs::read_dir(root)?
        .flatten()
        .take(MAX_LOCAL_FILES)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_file())
                .and_then(|_| entry.file_name().to_str().map(str::to_ascii_lowercase))
                .filter(|name| name.ends_with(".dll"))
        })
        .collect::<Vec<_>>();
    local_libraries.sort();
    local_libraries.dedup();
    let mut libraries = graphics_evidence.imported_libraries.clone();
    libraries.extend(local_libraries.iter().cloned());
    let mut input = BTreeSet::new();
    let mut audio = BTreeSet::new();
    let mut prerequisites = BTreeSet::new();
    for library in libraries {
        classify_compatibility_library(&library, &mut input, &mut audio, &mut prerequisites);
    }
    Ok(PeCompatibilityEvidence {
        graphics: graphics_evidence.apis,
        input,
        audio,
        prerequisites,
        imported_libraries: graphics_evidence.imported_libraries,
        local_libraries,
    })
}

fn classify_compatibility_library(
    library: &str,
    input: &mut BTreeSet<PeInputApi>,
    audio: &mut BTreeSet<PeAudioApi>,
    prerequisites: &mut BTreeSet<PePrerequisite>,
) {
    let library = library.to_ascii_lowercase();
    if library.starts_with("xinput") {
        input.insert(PeInputApi::XInput);
    }
    if library == "dinput.dll" || library == "dinput8.dll" {
        input.insert(PeInputApi::DirectInput);
    }
    if library.starts_with("sdl") {
        input.insert(PeInputApi::Sdl);
        audio.insert(PeAudioApi::Sdl);
    }
    if library.starts_with("xaudio")
        || library.starts_with("x3daudio")
        || library.starts_with("xapofx")
        || library.starts_with("xactengine")
    {
        audio.insert(PeAudioApi::XAudio);
    }
    if library == "dsound.dll" {
        audio.insert(PeAudioApi::DirectSound);
    }
    if library == "openal32.dll" {
        audio.insert(PeAudioApi::OpenAl);
    }
    if library.starts_with("fmod") {
        audio.insert(PeAudioApi::Fmod);
    }
    if library == "mss32.dll" || library == "mss64.dll" {
        audio.insert(PeAudioApi::Miles);
    }
    if library == "winmm.dll" {
        audio.insert(PeAudioApi::WinMm);
    }
    if library.starts_with("d3dx") || library.starts_with("d3dcompiler_4") {
        prerequisites.insert(PePrerequisite::LegacyDirectX);
    }
    if library.starts_with("msvcr")
        || library.starts_with("msvcp")
        || library.starts_with("vcruntime")
    {
        prerequisites.insert(PePrerequisite::VisualCpp);
    }
    if library.starts_with("physx") || library.starts_with("nxcooking") {
        prerequisites.insert(PePrerequisite::PhysX);
    }
}

fn graphics_api_for_library(name: &str) -> Option<PeGraphicsApi> {
    if name == "d3d8.dll" || name.starts_with("d3dx8_") {
        Some(PeGraphicsApi::D3d8)
    } else if name == "d3d9.dll" || name.starts_with("d3dx9_") {
        Some(PeGraphicsApi::D3d9)
    } else if name == "d3d10.dll"
        || name == "d3d10_1.dll"
        || name == "d3d10core.dll"
        || name.starts_with("d3dx10_")
    {
        Some(PeGraphicsApi::D3d10)
    } else if name == "d3d11.dll" || name.starts_with("d3dx11_") {
        Some(PeGraphicsApi::D3d11)
    } else if name == "d3d12.dll" {
        Some(PeGraphicsApi::D3d12)
    } else if name == "opengl32.dll" {
        Some(PeGraphicsApi::OpenGl)
    } else {
        None
    }
}

fn find_case_insensitive_child(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    fs::read_dir(root).ok()?.flatten().find_map(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
            .then(|| entry.path())
    })
}

fn pe_imports_from_bytes(bytes: &[u8]) -> io::Result<Vec<String>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid PE import table");
    if bytes.len() < 64 || !bytes.starts_with(b"MZ") {
        return Err(invalid());
    }
    let pe_offset = read_le_u32_at(bytes, 0x3c).ok_or_else(invalid)? as usize;
    if bytes.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
        return Err(invalid());
    }
    let coff = pe_offset + 4;
    let section_count = read_le_u16_at(bytes, coff + 2).ok_or_else(invalid)? as usize;
    let optional_size = read_le_u16_at(bytes, coff + 16).ok_or_else(invalid)? as usize;
    let optional = coff + 20;
    let data_directories = match read_le_u16_at(bytes, optional).ok_or_else(invalid)? {
        0x10b => optional + 96,
        0x20b => optional + 112,
        _ => return Err(invalid()),
    };
    let import_rva = read_le_u32_at(bytes, data_directories + 8).ok_or_else(invalid)?;
    if import_rva == 0 {
        return Ok(Vec::new());
    }
    let sections = optional.checked_add(optional_size).ok_or_else(invalid)?;
    let rva_to_offset = |rva: u32| -> Option<usize> {
        for index in 0..section_count {
            let section = sections.checked_add(index.checked_mul(40)?)?;
            let virtual_size = read_le_u32_at(bytes, section + 8)?;
            let virtual_address = read_le_u32_at(bytes, section + 12)?;
            let raw_size = read_le_u32_at(bytes, section + 16)?;
            let raw_offset = read_le_u32_at(bytes, section + 20)?;
            let span = virtual_size.max(raw_size);
            if rva >= virtual_address && rva < virtual_address.checked_add(span)? {
                return (raw_offset as usize).checked_add((rva - virtual_address) as usize);
            }
        }
        None
    };
    let mut descriptor = rva_to_offset(import_rva).ok_or_else(invalid)?;
    let mut imports = BTreeSet::new();
    for _ in 0..4096 {
        let entry = bytes.get(descriptor..descriptor + 20).ok_or_else(invalid)?;
        if entry.iter().all(|byte| *byte == 0) {
            return Ok(imports.into_iter().collect());
        }
        let name_rva = u32::from_le_bytes(entry[12..16].try_into().unwrap());
        let name_offset = rva_to_offset(name_rva).ok_or_else(invalid)?;
        let tail = bytes.get(name_offset..).ok_or_else(invalid)?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(invalid)?;
        if end == 0 || end > 260 {
            return Err(invalid());
        }
        let name = std::str::from_utf8(&tail[..end]).map_err(|_| invalid())?;
        imports.insert(name.to_owned());
        descriptor = descriptor.checked_add(20).ok_or_else(invalid)?;
    }
    Err(invalid())
}

fn read_le_u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_le_u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub fn inspect_macho_file_type(path: &Path) -> io::Result<Option<MachOFileType>> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 32];
    if file.read(&mut header)? < 16 {
        return Ok(None);
    }
    let (slice_offset, fat_endian, fat64) = match &header[..4] {
        [0xca, 0xfe, 0xba, 0xbe] => (Some(8_u64), Some(Endian::Big), false),
        [0xbe, 0xba, 0xfe, 0xca] => (Some(8_u64), Some(Endian::Little), false),
        [0xca, 0xfe, 0xba, 0xbf] => (Some(8_u64), Some(Endian::Big), true),
        [0xbf, 0xba, 0xfe, 0xca] => (Some(8_u64), Some(Endian::Little), true),
        _ => (None, None, false),
    };
    if let (Some(arch_offset), Some(endian)) = (slice_offset, fat_endian) {
        let offset = if fat64 {
            read_u64(
                &header[arch_offset as usize + 8..arch_offset as usize + 16],
                endian,
            )
        } else {
            read_u32(
                &header[arch_offset as usize + 8..arch_offset as usize + 12],
                endian,
            ) as u64
        };
        file.seek(SeekFrom::Start(offset))?;
        header.fill(0);
        if file.read(&mut header)? < 16 {
            return Ok(None);
        }
    }
    let endian = match &header[..4] {
        [0xfe, 0xed, 0xfa, 0xce] | [0xfe, 0xed, 0xfa, 0xcf] => Endian::Big,
        [0xce, 0xfa, 0xed, 0xfe] | [0xcf, 0xfa, 0xed, 0xfe] => Endian::Little,
        _ => return Ok(None),
    };
    let file_type = read_u32(&header[12..16], endian);
    Ok(Some(match file_type {
        2 => MachOFileType::Executable,
        6 => MachOFileType::Dylib,
        8 => MachOFileType::Bundle,
        other => MachOFileType::Other(other),
    }))
}

pub fn inspect_bytes(bytes: &[u8]) -> BinaryKind {
    if bytes.starts_with(b"MZ") {
        return BinaryKind::PortableExecutable;
    }
    if bytes.len() < 8 {
        return BinaryKind::Unknown;
    }

    let magic = &bytes[..4];
    match magic {
        [0xfe, 0xed, 0xfa, 0xce] | [0xfe, 0xed, 0xfa, 0xcf] => {
            BinaryKind::MachO(vec![architecture(read_u32(&bytes[4..8], Endian::Big))])
        }
        [0xce, 0xfa, 0xed, 0xfe] | [0xcf, 0xfa, 0xed, 0xfe] => {
            BinaryKind::MachO(vec![architecture(read_u32(&bytes[4..8], Endian::Little))])
        }
        [0xca, 0xfe, 0xba, 0xbe] => inspect_fat(bytes, Endian::Big, 20),
        [0xbe, 0xba, 0xfe, 0xca] => inspect_fat(bytes, Endian::Little, 20),
        [0xca, 0xfe, 0xba, 0xbf] => inspect_fat(bytes, Endian::Big, 24),
        [0xbf, 0xba, 0xfe, 0xca] => inspect_fat(bytes, Endian::Little, 24),
        _ => BinaryKind::Unknown,
    }
}

#[derive(Clone, Copy)]
enum Endian {
    Big,
    Little,
}

fn inspect_fat(bytes: &[u8], endian: Endian, entry_size: usize) -> BinaryKind {
    let count = read_u32(&bytes[4..8], endian) as usize;
    if count == 0 || count > 64 {
        return BinaryKind::Unknown;
    }
    let Some(required) = 8_usize.checked_add(count.saturating_mul(entry_size)) else {
        return BinaryKind::Unknown;
    };
    if bytes.len() < required {
        return BinaryKind::Unknown;
    }
    let mut architectures = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 8 + index * entry_size;
        architectures.push(architecture(read_u32(&bytes[offset..offset + 4], endian)));
    }
    architectures.sort_unstable();
    architectures.dedup();
    BinaryKind::MachO(architectures)
}

fn read_u32(bytes: &[u8], endian: Endian) -> u32 {
    let bytes: [u8; 4] = bytes.try_into().expect("four-byte slice");
    match endian {
        Endian::Big => u32::from_be_bytes(bytes),
        Endian::Little => u32::from_le_bytes(bytes),
    }
}

fn read_u64(bytes: &[u8], endian: Endian) -> u64 {
    let bytes: [u8; 8] = bytes.try_into().expect("eight-byte slice");
    match endian {
        Endian::Big => u64::from_be_bytes(bytes),
        Endian::Little => u64::from_le_bytes(bytes),
    }
}

fn architecture(cpu_type: u32) -> Architecture {
    match cpu_type {
        CPU_TYPE_X86 => Architecture::I386,
        CPU_TYPE_X86_64 => Architecture::X86_64,
        CPU_TYPE_ARM64 => Architecture::Arm64,
        other => Architecture::Other(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_i386_thin_macho() {
        let bytes = [0xce, 0xfa, 0xed, 0xfe, 7, 0, 0, 0];
        assert_eq!(
            inspect_bytes(&bytes),
            BinaryKind::MachO(vec![Architecture::I386])
        );
    }

    #[test]
    fn recognizes_universal_macho() {
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2];
        bytes.extend_from_slice(&CPU_TYPE_X86_64.to_be_bytes());
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(&CPU_TYPE_ARM64.to_be_bytes());
        bytes.extend_from_slice(&[0; 16]);
        assert_eq!(
            inspect_bytes(&bytes),
            BinaryKind::MachO(vec![Architecture::X86_64, Architecture::Arm64])
        );
    }

    #[test]
    fn recognizes_pe_payload() {
        assert_eq!(
            inspect_bytes(b"MZ-not-validated-beyond-format-marker"),
            BinaryKind::PortableExecutable
        );
    }

    #[test]
    fn reads_pe_machine_type() {
        let path = std::env::temp_dir().join(format!(
            "macroton-pe-machine-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let mut bytes = vec![0_u8; 0x86];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            inspect_pe_architecture(&path).unwrap(),
            Some(PeArchitecture::I386)
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_pe_import_table_and_classifies_graphics_api() {
        let mut bytes = vec![0_u8; 0x500];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        bytes[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&0xe0_u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x10b_u16.to_le_bytes());
        bytes[0x98 + 96 + 8..0x98 + 96 + 12].copy_from_slice(&0x1100_u32.to_le_bytes());
        let section = 0x98 + 0xe0;
        bytes[section..section + 6].copy_from_slice(b".rdata");
        bytes[section + 8..section + 12].copy_from_slice(&0x300_u32.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x300_u32.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[0x300 + 12..0x300 + 16].copy_from_slice(&0x1180_u32.to_le_bytes());
        bytes[0x380..0x389].copy_from_slice(b"d3d9.dll\0");

        assert_eq!(
            pe_imports_from_bytes(&bytes).unwrap(),
            vec!["d3d9.dll".to_owned()]
        );
        assert_eq!(
            graphics_api_for_library("d3dx9_43.dll"),
            Some(PeGraphicsApi::D3d9)
        );
        assert_eq!(
            graphics_api_for_library("opengl32.dll"),
            Some(PeGraphicsApi::OpenGl)
        );
    }

    #[test]
    fn classifies_legacy_game_input_audio_and_prerequisites() {
        let mut input = BTreeSet::new();
        let mut audio = BTreeSet::new();
        let mut prerequisites = BTreeSet::new();
        for library in [
            "xinput1_3.dll",
            "dinput8.dll",
            "xaudio2_7.dll",
            "PhysXLoader.dll",
            "d3dx9_43.dll",
            "msvcr90.dll",
        ] {
            classify_compatibility_library(library, &mut input, &mut audio, &mut prerequisites);
        }
        assert_eq!(
            input,
            BTreeSet::from([PeInputApi::XInput, PeInputApi::DirectInput])
        );
        assert_eq!(audio, BTreeSet::from([PeAudioApi::XAudio]));
        assert_eq!(
            prerequisites,
            BTreeSet::from([
                PePrerequisite::LegacyDirectX,
                PePrerequisite::VisualCpp,
                PePrerequisite::PhysX,
            ])
        );
    }

    #[test]
    fn reads_macho_executable_file_type() {
        let path = std::env::temp_dir().join(format!(
            "macroton-macho-type-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let mut bytes = vec![0_u8; 32];
        bytes[..4].copy_from_slice(&[0xce, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(&7_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            inspect_macho_file_type(&path).unwrap(),
            Some(MachOFileType::Executable)
        );
        std::fs::remove_file(path).unwrap();
    }
}
