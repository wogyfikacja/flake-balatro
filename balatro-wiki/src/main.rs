use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Parser)]
#[command(name = "balatro-wiki")]
#[command(about = "A CLI tool for browsing and searching Balatro mods from the wiki")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse mods by category
    Browse {
        /// Category to browse (content, joker, qol, crossover, technical, api)
        category: Option<String>,
    },
    /// Search for mods by name or description
    Search {
        /// Search query
        query: String,
    },
    /// Get detailed information about a specific mod
    Info {
        /// Mod name
        name: String,
    },
    /// List all available categories
    Categories,
    /// Update the local mod database
    Update,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ModInfo {
    name: String,
    description: String,
    author: Option<String>,
    version: Option<String>,
    github_url: Option<String>,
    wiki_url: String,
    category: String,
    dependencies: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModDatabase {
    mods: HashMap<String, ModInfo>,
    categories: HashMap<String, Vec<String>>,
    last_updated: String,
}

const WIKI_BASE_URL: &str = "https://balatromods.miraheze.org";
const CACHE_FILE: &str = "~/.cache/balatro-wiki/mods.json";

impl ModDatabase {
    fn new() -> Self {
        Self {
            mods: HashMap::new(),
            categories: HashMap::new(),
            last_updated: Utc::now().to_rfc3339(),
        }
    }

    fn load_or_create() -> Result<Self> {
        let cache_path = shellexpand::tilde(CACHE_FILE);
        let cache_path = std::path::Path::new(cache_path.as_ref());
        
        if cache_path.exists() {
            let content = std::fs::read_to_string(cache_path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::new())
        }
    }

    fn should_update(&self) -> bool {
        if self.mods.is_empty() {
            return true;
        }
        
        // Parse last_updated timestamp
        if let Ok(last_updated) = chrono::DateTime::parse_from_rfc3339(&self.last_updated) {
            let now = Utc::now();
            let age = now.signed_duration_since(last_updated.with_timezone(&Utc));
            
            // Update if older than 24 hours
            age.num_hours() >= 24
        } else {
            true // Invalid timestamp, force update
        }
    }

    async fn ensure_fresh(scraper: &WikiScraper) -> Result<Self> {
        Self::ensure_fresh_with_verbosity(scraper, true).await
    }
    
    async fn ensure_fresh_silent(scraper: &WikiScraper) -> Result<Self> {
        Self::ensure_fresh_with_verbosity(scraper, false).await
    }
    
    async fn ensure_fresh_with_verbosity(scraper: &WikiScraper, verbose: bool) -> Result<Self> {
        let mut db = Self::load_or_create()?;
        
        if db.should_update() {
            if verbose {
                println!("🔄 Updating mod database...");
            }
            db = scraper.update_database_with_verbosity(verbose).await?;
            db.save()?;
            if verbose {
                println!("✅ Database updated with {} mods", db.mods.len());
            }
        }
        
        Ok(db)
    }

    fn save(&self) -> Result<()> {
        let cache_path = shellexpand::tilde(CACHE_FILE);
        let cache_path = std::path::Path::new(cache_path.as_ref());
        
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(cache_path, content)?;
        Ok(())
    }
}

struct WikiScraper {
    client: Client,
}

impl WikiScraper {
    fn new() -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
            
        Self { client }
    }

    async fn scrape_category_page(&self, category: &str) -> Result<Vec<String>> {
        self.scrape_category_page_with_verbosity(category, true).await
    }
    
    async fn scrape_category_page_with_verbosity(&self, category: &str, verbose: bool) -> Result<Vec<String>> {
        // Use MediaWiki API instead of HTML scraping
        let api_url = format!("{}/w/api.php?action=query&list=categorymembers&cmtitle=Category:{}&format=json&cmlimit=50", 
                             WIKI_BASE_URL, category);
        if verbose {
            println!("  API request: {}", api_url);
        }
        
        let response = self.client
            .get(&api_url)
            .header("Accept", "application/json")
            .send()
            .await?;
            
        let json_text = response.text().await?;
        if verbose {
            println!("  Got {} bytes of JSON", json_text.len());
        }
        
        // Parse JSON response
        let json: serde_json::Value = serde_json::from_str(&json_text)?;
        
        let mut mod_names = Vec::new();
        
        if let Some(query) = json.get("query") {
            if let Some(categorymembers) = query.get("categorymembers") {
                if let Some(members) = categorymembers.as_array() {
                    if verbose {
                        println!("  Found {} category members", members.len());
                    }
                    
                    for member in members {
                        if let Some(title) = member.get("title").and_then(|t| t.as_str()) {
                            // Skip category pages and other namespace pages
                            if !title.contains("Category:") && !title.contains("File:") && !title.contains("Template:") {
                                mod_names.push(title.to_string());
                                if verbose {
                                    println!("    ✓ {}", title);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        if verbose {
            println!("  Extracted {} mod names", mod_names.len());
        }
        Ok(mod_names)
    }

    async fn scrape_mod_page(&self, mod_name: &str) -> Result<ModInfo> {
        let url = format!("{}/wiki/{}", WIKI_BASE_URL, mod_name);
        let response = self.client.get(&url).send().await?;
        let html = response.text().await?;
        let document = Html::parse_document(&html);
        
        // Extract basic info
        let title_selector = Selector::parse("h1.firstHeading").unwrap();
        let name = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_else(|| mod_name.to_string());

        // Extract description from multiple sources
        let description = extract_description(&document);

        // Look for GitHub links
        let link_selector = Selector::parse("a[href*='github.com']").unwrap();
        let github_url = document
            .select(&link_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .map(|s| s.to_string());

        // Extract from infobox if present
        let infobox_selector = Selector::parse(".infobox tr").unwrap();
        let mut author = None;
        let mut version = None;
        
        for row in document.select(&infobox_selector) {
            let text = row.text().collect::<String>();
            if text.to_lowercase().contains("author") {
                // Extract author from next sibling or same row
                author = Some("Unknown".to_string()); // Simplified for now
            }
            if text.to_lowercase().contains("version") {
                version = Some("Unknown".to_string()); // Simplified for now
            }
        }

        Ok(ModInfo {
            name,
            description: description.trim().to_string(),
            author,
            version,
            github_url,
            wiki_url: url,
            category: "Unknown".to_string(), // Will be set by caller
            dependencies: Vec::new(),
        })
    }

    async fn update_database(&self) -> Result<ModDatabase> {
        self.update_database_with_verbosity(true).await
    }
    
    async fn update_database_with_verbosity(&self, verbose: bool) -> Result<ModDatabase> {
        let mut db = ModDatabase::new();
        
        let categories = vec![
            ("Content Mods", "Content%20Mods"),
            ("Joker Mods", "Joker%20Mods"),
            ("Quality of Life Mods", "Quality%20of%20Life%20Mods"),
            ("Crossover Mods", "Crossover%20Mods"),
            ("Technical Mods", "Technical%20Mods"),
            ("API Mods", "API%20Mods"),
        ];

        // Collect all mod names from all categories first
        let mut all_mod_names = std::collections::HashSet::new();
        let mut mod_categories = std::collections::HashMap::new();
        
        for (category_name, wiki_category) in &categories {
            if verbose {
                println!("Collecting mods from category: {}", category_name);
            }
            
            match self.scrape_category_page_with_verbosity(wiki_category, verbose).await {
                Ok(mod_names) => {
                    for mod_name in mod_names {
                        all_mod_names.insert(mod_name.clone());
                        mod_categories.insert(mod_name, category_name.to_string());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to scrape category {}: {}", category_name, e);
                }
            }
        }
        
        if verbose {
            println!("Processing {} unique mods concurrently...", all_mod_names.len());
        }
        
        // Process all mods concurrently
        let mut handles = Vec::new();
        for mod_name in all_mod_names.iter() {
            let client = self.client.clone();
            let name = mod_name.clone();
            let handle = tokio::spawn(async move {
                let scraper = WikiScraper { client };
                (name.clone(), scraper.scrape_mod_page(&name).await)
            });
            handles.push(handle);
        }
        
        // Collect results and organize by category
        let mut category_mods: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for (category_name, _) in &categories {
            category_mods.insert(category_name.to_string(), Vec::new());
        }
        
        for handle in handles {
            match handle.await {
                Ok((mod_name, result)) => {
                    match result {
                        Ok(mut mod_info) => {
                            if let Some(category) = mod_categories.get(&mod_name) {
                                mod_info.category = category.to_string();
                                if let Some(cat_mods) = category_mods.get_mut(category) {
                                    cat_mods.push(mod_info.name.clone());
                                }
                                db.mods.insert(mod_info.name.clone(), mod_info);
                                if verbose {
                                    println!("  ✓ {}", mod_name);
                                }
                            }
                        }
                        Err(e) => {
                            if verbose {
                                eprintln!("  ✗ Failed to scrape {}: {}", mod_name, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    if verbose {
                        eprintln!("  ✗ Task failed: {}", e);
                    }
                }
            }
        }
        
        db.categories = category_mods;
        Ok(db)
    }
}

async fn browse_mods(db: &ModDatabase, category: Option<String>) -> Result<()> {
    match category {
        Some(cat) => {
            if let Some(mod_names) = db.categories.get(&cat) {
                println!("🎮 {} ({} mods):", cat, mod_names.len());
                println!("{}", "─".repeat(50));
                
                for mod_name in mod_names {
                    if let Some(mod_info) = db.mods.get(mod_name) {
                        println!("🃏 {}", mod_info.name);
                        println!("   {}", truncate(&mod_info.description, 300));
                        if let Some(author) = &mod_info.author {
                            println!("   👤 by {}", author);
                        }
                        if let Some(github) = &mod_info.github_url {
                            println!("   🔗 {}", github);
                        }
                        println!();
                    }
                }
            } else {
                println!("Category '{}' not found. Available categories:", cat);
                list_categories(db);
            }
        }
        None => {
            println!("📦 All Balatro Mods ({} total):", db.mods.len());
            println!("{}", "─".repeat(50));
            
            for category in db.categories.keys() {
                let count = db.categories.get(category).map(|v| v.len()).unwrap_or(0);
                println!("🗂️  {} ({} mods)", category, count);
            }
            println!("\nUse 'browse <category>' to see mods in a specific category");
        }
    }
    Ok(())
}

fn search_mods(db: &ModDatabase, query: &str) -> Result<()> {
    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();
    
    for mod_info in db.mods.values() {
        let score = calculate_search_score(mod_info, &query_lower);
        if score > 0 {
            matches.push((mod_info, score));
        }
    }
    
    matches.sort_by(|a, b| b.1.cmp(&a.1));
    
    if matches.is_empty() {
        println!("No mods found matching '{}'", query);
        return Ok(());
    }
    
    println!("🔍 Search results for '{}' ({} matches):", query, matches.len());
    println!("{}", "─".repeat(50));
    
    for (mod_info, _score) in matches.iter().take(20) {
        println!("🃏 {}", mod_info.name);
        println!("   📁 {}", mod_info.category);
        println!("   {}", truncate(&mod_info.description, 300));
        if let Some(github) = &mod_info.github_url {
            println!("   🔗 {}", github);
        }
        println!();
    }
    
    Ok(())
}

fn show_mod_info(db: &ModDatabase, name: &str) -> Result<()> {
    let mod_info = db.mods.values()
        .find(|m| m.name.to_lowercase() == name.to_lowercase())
        .ok_or_else(|| anyhow!("Mod '{}' not found", name))?;
    
    println!("🃏 {}", mod_info.name);
    println!("{}", "═".repeat(50));
    println!("📁 Category: {}", mod_info.category);
    println!("📝 Description: {}", mod_info.description);
    
    if let Some(author) = &mod_info.author {
        println!("👤 Author: {}", author);
    }
    
    if let Some(version) = &mod_info.version {
        println!("📦 Version: {}", version);
    }
    
    if let Some(github) = &mod_info.github_url {
        println!("🔗 GitHub: {}", github);
        println!("\n💾 To install this mod:");
        println!("   balatro-install-mod {}", github);
    }
    
    println!("🌐 Wiki: {}", mod_info.wiki_url);
    
    if !mod_info.dependencies.is_empty() {
        println!("🔗 Dependencies: {}", mod_info.dependencies.join(", "));
    }
    
    Ok(())
}

fn list_categories(db: &ModDatabase) {
    println!("📂 Available categories:");
    for (category, mods) in &db.categories {
        println!("  {} ({} mods)", category, mods.len());
    }
}

fn calculate_search_score(mod_info: &ModInfo, query: &str) -> i32 {
    let mut score = 0;
    
    // Exact name match gets highest score
    if mod_info.name.to_lowercase() == query {
        score += 100;
    } else if mod_info.name.to_lowercase().contains(query) {
        score += 50;
    }
    
    // Description match
    if mod_info.description.to_lowercase().contains(query) {
        score += 25;
    }
    
    // Author match
    if let Some(author) = &mod_info.author {
        if author.to_lowercase().contains(query) {
            score += 20;
        }
    }
    
    // Category match
    if mod_info.category.to_lowercase().contains(query) {
        score += 15;
    }
    
    score
}

fn extract_description(document: &Html) -> String {
    let mut description_parts = Vec::new();
    
    // Try infobox description first
    let infobox_selector = Selector::parse(".infobox tr").unwrap();
    for row in document.select(&infobox_selector) {
        let cells: Vec<_> = row.select(&Selector::parse("td").unwrap()).collect();
        if cells.len() >= 2 {
            let header_text = cells[0].text().collect::<String>().to_lowercase();
            if header_text.contains("description") {
                let desc_text = cells[1].text().collect::<Vec<_>>().join(" ");
                let cleaned = clean_text(&desc_text);
                if cleaned.len() > 10 && !cleaned.starts_with("http") && !cleaned.contains("github.com") {
                    description_parts.push(cleaned);
                }
            }
        }
    }
    
    // Extract multiple meaningful paragraphs from main content
    let para_selector = Selector::parse("div.mw-parser-output > p").unwrap();
    for para in document.select(&para_selector) {
        let text = para.text().collect::<Vec<_>>().join(" ");
        let cleaned = clean_text(&text);
        if cleaned.len() > 20 
            && !cleaned.chars().all(|c| c.is_whitespace()) 
            && !cleaned.starts_with("http")
            && !cleaned.contains("github.com")
            && !cleaned.contains("gamebanana.com")
            && !cleaned.contains("drive.google.com")
            && !cleaned.to_lowercase().contains("disambiguation")
            && !cleaned.to_lowercase().contains("redirect")
            && !cleaned.to_lowercase().contains("this article is a stub")
            && !cleaned.to_lowercase().contains("bibliography")
            && !cleaned.to_lowercase().contains("references")
            && !cleaned.to_lowercase().contains("external links")
            && !cleaned.to_lowercase().contains("see also")
            && !cleaned.to_lowercase().contains("categories")
            && !cleaned.to_lowercase().contains("navigation")
            && !cleaned.contains("2.1")
            && !cleaned.contains("2.2")
            && !cleaned.contains("2.3") {
            description_parts.push(cleaned);
            // Collect up to 3 meaningful paragraphs for fuller descriptions
            if description_parts.len() >= 3 {
                break;
            }
        }
    }
    
    // Try list items for feature descriptions
    let list_selector = Selector::parse("div.mw-parser-output ul li").unwrap();
    let mut features = Vec::new();
    for item in document.select(&list_selector) {
        let text = item.text().collect::<Vec<_>>().join(" ");
        let cleaned = clean_text(&text);
        if cleaned.len() > 15 
            && !cleaned.starts_with("http") 
            && !cleaned.contains("github.com")
            && (cleaned.to_lowercase().contains("adds") 
                || cleaned.to_lowercase().contains("features")
                || cleaned.to_lowercase().contains("includes")
                || cleaned.to_lowercase().contains("joker")) {
            features.push(cleaned);
            if features.len() >= 2 {
                break;
            }
        }
    }
    
    // Combine all parts
    if !features.is_empty() {
        description_parts.extend(features);
    }
    
    let combined = description_parts.join(" ");
    if combined.len() > 10 {
        return truncate(&combined, 500); // Increased from 200 to 500
    }
    
    // Try any div with text content as fallback
    let content_selector = Selector::parse("div.mw-parser-output div, div.mw-parser-output li").unwrap();
    for element in document.select(&content_selector) {
        let text = element.text().collect::<Vec<_>>().join(" ");
        let cleaned = clean_text(&text);
        if cleaned.len() > 30 
            && !cleaned.starts_with("http")
            && !cleaned.contains("github.com")
            && !cleaned.contains("gamebanana.com")
            && !cleaned.contains("drive.google.com")
            && !cleaned.to_lowercase().contains("navigation")
            && !cleaned.to_lowercase().contains("categories")
            && !cleaned.to_lowercase().contains("this article is a stub") {
            return truncate(&cleaned, 500);
        }
    }
    
    "No description available".to_string()
}

fn clean_text(text: &str) -> String {
    text.trim()
        .split_whitespace()
        .filter(|word| !word.starts_with("http") && !word.contains("github.com") && !word.contains("gamebanana.com"))
        .collect::<Vec<_>>()
        .join(" ")
        .replace("[[", "")
        .replace("]]", "")
        .replace("{{", "")
        .replace("}}", "")
        .replace("()", "")
        .replace("  ", " ")
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        let mut result = String::new();
        let mut char_count = 0;
        
        for c in s.chars() {
            if char_count + 3 >= max_len {
                break;
            }
            result.push(c);
            char_count += 1;
        }
        result.push_str("...");
        result
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Update => {
            println!("🔄 Updating mod database from wiki...");
            let scraper = WikiScraper::new();
            let db = scraper.update_database().await?;
            db.save()?;
            println!("✅ Database updated with {} mods", db.mods.len());
        }
        _ => {
            let scraper = WikiScraper::new();
            let db = ModDatabase::ensure_fresh_silent(&scraper).await?;
            
            match cli.command {
                Commands::Browse { category } => {
                    browse_mods(&db, category).await?;
                }
                Commands::Search { query } => {
                    search_mods(&db, &query)?;
                }
                Commands::Info { name } => {
                    show_mod_info(&db, &name)?;
                }
                Commands::Categories => {
                    list_categories(&db);
                }
                Commands::Update => unreachable!(),
            }
        }
    }
    
    Ok(())
}