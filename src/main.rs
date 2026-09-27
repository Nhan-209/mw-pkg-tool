mod pkg;

use anyhow::Result;
use clap::{Parser, Subcommand};
use pkg::{repack_pkg, unpack_pkg, PackOptions};
use std::env;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "mw-pkg-tool")]
#[command(author = "Nhan-209")]
#[command(version = "0.1.0")]
#[command(about = "Ultra-fast native tool to unpack and repack Mini World .PKG archives", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Drag & Drop input path (file or folder)
    #[arg(value_name = "PATH")]
    input_path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Unpack a .pkg archive into a folder
    Unpack {
        /// Path to .pkg file
        input: PathBuf,

        /// Custom output folder (defaults to <name>_extracted)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Repack a folder back into a .pkg archive
    Pack {
        /// Path to folder containing extracted files
        input: PathBuf,

        /// Custom output .pkg path (defaults to <folder_name>.pkg)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Force LZ4 compression on all files
        #[arg(long)]
        force_compress: bool,

        /// Disable compression on all files
        #[arg(long)]
        no_compress: bool,
    },

    /// Automatically scan %APPDATA% for Mini World .pkg files
    Scan,
}

fn pause_for_user() {
    println!("\nPress Enter to exit...");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

fn scan_miniworld_pkgs() -> Vec<PathBuf> {
    let mut pkgs = Vec::new();
    let mut search_dirs = Vec::new();

    if let Ok(curr) = env::current_dir() {
        search_dirs.push(curr.clone());
        search_dirs.push(curr.join("pkg_assets"));
    }

    if let Ok(appdata) = env::var("APPDATA") {
        let appdata_path = PathBuf::from(appdata);
        if let Ok(entries) = std::fs::read_dir(&appdata_path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains("miniword") || name.contains("miniworld") {
                    let d = entry.path();
                    search_dirs.push(d.clone());
                    search_dirs.push(d.join("pkg_assets"));
                    search_dirs.push(d.join("AssetsCache"));
                }
            }
        }
    }

    for dir in search_dirs {
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext) = path.extension() {
                            if ext.eq_ignore_ascii_case("pkg") {
                                if !pkgs.contains(&path) {
                                    pkgs.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pkgs
}

fn run_interactive_menu() -> Result<()> {
    println!("================================================================");
    println!("      MINI WORLD .PKG ARCHIVE TOOL - RUST HIGH-PERFORMANCE");
    println!("================================================================");
    println!("1. Unpack a .pkg file");
    println!("2. Repack a directory to .pkg");
    println!("3. Auto-scan Mini World folders in %APPDATA%");
    println!("4. Exit");
    println!("================================================================");
    print!("Choose an option [1-4]: ");
    io::stdout().flush()?;

    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let choice = choice.trim();

    match choice {
        "1" => {
            print!("\nEnter path to .pkg file: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let trimmed = input.trim().trim_matches('"');
            if !trimmed.is_empty() {
                unpack_pkg(trimmed, None::<PathBuf>)?;
            }
        }
        "2" => {
            print!("\nEnter path to folder to repack: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let trimmed = input.trim().trim_matches('"');
            if !trimmed.is_empty() {
                repack_pkg(trimmed, None::<PathBuf>, PackOptions::default())?;
            }
        }
        "3" => {
            println!("\n[*] Scanning for .pkg files in system...");
            let pkgs = scan_miniworld_pkgs();
            if pkgs.is_empty() {
                println!("[!] No .pkg files found in standard locations.");
            } else {
                println!("[+] Found {} PKG archive(s):", pkgs.len());
                for (idx, p) in pkgs.iter().enumerate() {
                    let sz = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                    println!(
                        "  [{}] {:<25} ({:.2} MB) -> {:?}",
                        idx + 1,
                        p.file_name().unwrap_or_default().to_string_lossy(),
                        sz as f64 / 1_048_576.0,
                        p
                    );
                }
                print!("\nEnter number to unpack (or 'all' to unpack everything): ");
                io::stdout().flush()?;
                let mut sel = String::new();
                io::stdin().read_line(&mut sel)?;
                let sel = sel.trim();
                if sel.eq_ignore_ascii_case("all") {
                    for p in &pkgs {
                        let _ = unpack_pkg(p, None::<PathBuf>);
                    }
                } else if let Ok(num) = sel.parse::<usize>() {
                    if num >= 1 && num <= pkgs.len() {
                        let _ = unpack_pkg(&pkgs[num - 1], None::<PathBuf>);
                    }
                }
            }
        }
        _ => {
            println!("Exiting.");
            return Ok(());
        }
    }

    pause_for_user();
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // Case 1: Drag & drop directly onto .exe (args.len() == 2 and no subcommand flag)
    if args.len() == 2 && !args[1].starts_with('-') {
        let p = PathBuf::from(args[1].trim_matches('"'));
        if p.is_file() {
            println!("[*] Drag-and-drop detected: Unpacking PKG...");
            let res = unpack_pkg(&p, None::<PathBuf>);
            if let Err(e) = res {
                eprintln!("[!] Error: {:#}", e);
            }
            pause_for_user();
            return Ok(());
        } else if p.is_dir() {
            println!("[*] Drag-and-drop detected: Repacking folder into PKG...");
            let res = repack_pkg(&p, None::<PathBuf>, PackOptions::default());
            if let Err(e) = res {
                eprintln!("[!] Error: {:#}", e);
            }
            pause_for_user();
            return Ok(());
        }
    }

    // Case 2: Double-click with no arguments
    if args.len() <= 1 {
        return run_interactive_menu();
    }

    // Case 3: Standard CLI parser
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Unpack { input, output }) => {
            unpack_pkg(input, output)?;
        }
        Some(Commands::Pack {
            input,
            output,
            force_compress,
            no_compress,
        }) => {
            let options = PackOptions {
                force_compression: force_compress,
                no_compression: no_compress,
            };
            repack_pkg(input, output, options)?;
        }
        Some(Commands::Scan) => {
            let pkgs = scan_miniworld_pkgs();
            println!("Found {} PKG files:", pkgs.len());
            for p in pkgs {
                println!(" - {:?}", p);
            }
        }
        None => {
            if let Some(path) = cli.input_path {
                if path.is_file() {
                    unpack_pkg(path, None::<PathBuf>)?;
                } else if path.is_dir() {
                    repack_pkg(path, None::<PathBuf>, PackOptions::default())?;
                }
            } else {
                run_interactive_menu()?;
            }
        }
    }

    Ok(())
}
