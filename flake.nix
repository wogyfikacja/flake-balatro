{
  description = "Balatro modding environment with wiki integration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        
        # Rust wiki scraper for mod discovery
        balatro-wiki = pkgs.rustPlatform.buildRustPackage {
          pname = "balatro-wiki";
          version = "0.1.0";
          src = ./balatro-wiki;
          cargoLock.lockFile = ./balatro-wiki/Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.apple_sdk.frameworks.Security ];
        };
        
        # Steam detection script - finds Steam, Balatro, and mod paths
        steam-detector = pkgs.writeShellScript "steam-detector" ''
          find_steam_root() {
            for path in "$HOME/.local/share/Steam" "$HOME/.steam/steam" "$HOME/snap/steam/common/.local/share/Steam" "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam"; do
              [ -f "$path/config/libraryfolders.vdf" ] && echo "$path" && return 0
            done
            return 1
          }
          
          find_balatro_path() {
            local steam_root=$(find_steam_root) || return 1
            local vdf_file="$steam_root/config/libraryfolders.vdf"
            [ ! -f "$vdf_file" ] && return 1
            
            while IFS= read -r line; do
              if [[ "$line" =~ \"path\"[[:space:]]*\"([^\"]*)\" ]]; then
                local game_path="''${BASH_REMATCH[1]}/steamapps/common/Balatro"
                [ -f "$game_path/Balatro.exe" ] && echo "$game_path" && return 0
              fi
            done < "$vdf_file"
            
            echo "$steam_root/steamapps/common/Balatro"
          }
          
          find_balatro_mods_path() {
            local steam_root=$(find_steam_root)
            local compat_path="''${steam_root:-$HOME/.local/share/Steam}/steamapps/compatdata/2379780/pfx/drive_c/users/steamuser/AppData/Roaming/Balatro/Mods"
            echo "$compat_path"
          }
          
          case "$1" in
            root) find_steam_root ;;
            game) find_balatro_path ;;
            mods) find_balatro_mods_path ;;
            *) echo "Usage: $0 {root|game|mods}" >&2; exit 1 ;;
          esac
        '';
        
        # Core mod management scripts
        mod-scripts = pkgs.writeShellScriptBin "balatro-mod-setup" ''
          set -e
          BALATRO_MODS=$(${steam-detector} mods)
          GAME_PATH=$(${steam-detector} game)
          
          # Create and setup mod directory
          mkdir -p "$BALATRO_MODS"
          echo "Using mods directory: $BALATRO_MODS"
          
          # Install/update Steamodded
          if [ ! -d "$BALATRO_MODS/smods" ]; then
            echo "Installing Steamodded..."
            cd "$BALATRO_MODS" && git clone https://github.com/Steamodded/smods.git
          else
            echo "Updating Steamodded..."
            cd "$BALATRO_MODS/smods" && git pull
          fi
          
          # Setup lovely injector for Linux
          if [ -d "$GAME_PATH" ]; then
            cd "$GAME_PATH"
            if [ -f "lovely-x86_64-pc-windows-msvc.zip" ] && [ ! -f "liblovely.so" ]; then
              unzip -o "lovely-x86_64-pc-windows-msvc.zip" 2>/dev/null || true
            fi
            [ -f "run_lovely_linux.sh" ] && chmod +x "run_lovely_linux.sh"
          fi
          
          # Fix permissions
          find "$BALATRO_MODS" -type d -exec chmod 755 {} + 2>/dev/null || true
          find "$BALATRO_MODS" -type f -exec chmod 644 {} + 2>/dev/null || true
          echo "✅ Setup complete!"
        '';
        
        install-mod = pkgs.writeShellScriptBin "balatro-install-mod" ''
          set -e
          [ $# -eq 0 ] && echo "Usage: balatro-install-mod <url|path|zip>" && exit 1
          
          BALATRO_MODS=$(${steam-detector} mods)
          [ ! -d "$BALATRO_MODS" ] && echo "Run 'balatro-mod-setup' first" && exit 1
          
          cd "$BALATRO_MODS"
          
          if [[ "$1" == *.git ]] || [[ "$1" == *github.com* ]]; then
            MOD_NAME=$(basename "$1" .git)
            if [ -d "$MOD_NAME" ]; then
              cd "$MOD_NAME" && git pull
            else
              git clone "$1"
            fi
          elif [[ "$1" == *.zip ]]; then
            unzip -o "$1"
          elif [ -d "$1" ]; then
            cp -r "$1" "./$(basename "$1")"
          else
            echo "Unsupported format" && exit 1
          fi
          
          find "$BALATRO_MODS" -type d -exec chmod 755 {} + 2>/dev/null || true
          find "$BALATRO_MODS" -type f -exec chmod 644 {} + 2>/dev/null || true
          echo "✅ Installed!"
        '';
        
        list-mods = pkgs.writeShellScriptBin "balatro-list-mods" ''
          BALATRO_MODS=$(${steam-detector} mods)
          [ ! -d "$BALATRO_MODS" ] && echo "Run 'balatro-mod-setup' first" && exit 1
          
          echo "📦 Installed mods ($BALATRO_MODS):"
          cd "$BALATRO_MODS"
          for mod in */; do
            [ -d "$mod" ] && echo "🃏 $mod" && [ -f "$mod/README.md" ] && echo "   $(head -n1 "$mod/README.md" 2>/dev/null)"
          done
        '';
        
        remove-mod = pkgs.writeShellScriptBin "balatro-remove-mod" ''
          [ $# -eq 0 ] && echo "Usage: balatro-remove-mod <name>" && exit 1
          
          BALATRO_MODS=$(${steam-detector} mods)
          [ ! -d "$BALATRO_MODS" ] && echo "Mods directory not found" && exit 1
          
          MOD_DIR="$BALATRO_MODS/$1"
          if [ ! -d "$MOD_DIR" ]; then
            echo "Mod '$1' not found. Available:"
            ls -1 "$BALATRO_MODS"
            exit 1
          fi
          
          rm -rf "$MOD_DIR"
          echo "✅ Removed $1"
        '';
        
        launch-balatro = pkgs.writeShellScriptBin "balatro-launch" ''
          GAME_PATH=$(${steam-detector} game)
          BALATRO_MODS=$(${steam-detector} mods)
          
          [ ! -d "$GAME_PATH" ] && echo "Balatro not found. Install via Steam." && exit 1
          [ ! -d "$BALATRO_MODS" ] && echo "Run 'balatro-mod-setup' first" && exit 1
          
          cd "$GAME_PATH"
          mkdir -p ~/.config/love
          ln -sf "$BALATRO_MODS" ~/.config/love/Mods 2>/dev/null || true
          
          # Download latest lovely injector if needed
          if [ ! -f "liblovely.so" ]; then
            LATEST_URL=$(curl -s https://api.github.com/repos/ethangreen-dev/lovely-injector/releases/latest | grep "browser_download_url.*x86_64-unknown-linux-gnu.tar.gz" | cut -d '"' -f 4)
            curl -L "$LATEST_URL" -o lovely.tgz
            tar -xzf lovely.tgz && rm lovely.tgz && chmod +x liblovely.so
          fi
          
          LD_PRELOAD=./liblovely.so love Balatro.exe "$@"
        '';
        
        # Wiki integration wrapper scripts
        browse-mods = pkgs.writeShellScriptBin "balatro-browse-mods" ''
          ${balatro-wiki}/bin/balatro-wiki browse "''${1:-}"
        '';
        
        search-mods = pkgs.writeShellScriptBin "balatro-search-mods" ''
          [ $# -eq 0 ] && echo "Usage: balatro-search-mods <query>" && exit 1
          ${balatro-wiki}/bin/balatro-wiki search "$@"
        '';
        
        mod-info = pkgs.writeShellScriptBin "balatro-mod-info" ''
          [ $# -eq 0 ] && echo "Usage: balatro-mod-info <name>" && exit 1
          ${balatro-wiki}/bin/balatro-wiki info "$@"
        '';
        
        update-wiki = pkgs.writeShellScriptBin "balatro-update-wiki" ''
          ${balatro-wiki}/bin/balatro-wiki update
        '';
        
        install-from-wiki = pkgs.writeShellScriptBin "balatro-install-from-wiki" ''
          [ $# -eq 0 ] && echo "Usage: balatro-install-from-wiki <name>" && exit 1
          
          MOD_INFO=$(${balatro-wiki}/bin/balatro-wiki info "$@" 2>/dev/null) || {
            echo "Mod not found. Try: balatro-search-mods $*"
            exit 1
          }
          
          GITHUB_URL=$(echo "$MOD_INFO" | grep "🔗 GitHub:" | sed 's/🔗 GitHub: //')
          [ -z "$GITHUB_URL" ] && echo "No GitHub URL found" && exit 1
          
          ${install-mod}/bin/balatro-install-mod "$GITHUB_URL"
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Core tools
            python3 python3Packages.pip git curl jq
            
            # Archive handling  
            p7zip unzip zip
            
            # Lua/Love2D environment
            lua5_4 luarocks love
            
            # Utilities
            tree file findutils coreutils
            
            # Mod management
            mod-scripts install-mod list-mods remove-mod launch-balatro
            
            # Wiki integration
            browse-mods search-mods mod-info update-wiki install-from-wiki
            
            # Steam detection
            (pkgs.writeShellScriptBin "steam-detector" "exec ${steam-detector} \"$@\"")
          ];

          shellHook = ''
            echo "🎰 Balatro Modding Environment"
            echo "================================"
            echo "Mod Management:"
            echo "  balatro-mod-setup     - Setup/update Steamodded framework"
            echo "  balatro-install-mod   - Install mods (Git, zip, or local)"
            echo "  balatro-list-mods     - List installed mods"
            echo "  balatro-remove-mod    - Remove a mod"
            echo "  balatro-launch        - Launch Balatro with mods (Linux)"
            echo ""
            echo "Wiki Integration:"
            echo "  balatro-update-wiki      - Update mod database from wiki"
            echo "  balatro-browse-mods      - Browse mods by category"
            echo "  balatro-search-mods      - Search for mods"
            echo "  balatro-mod-info         - Get detailed mod information"
            echo "  balatro-install-from-wiki - Install mod directly from wiki"
            echo ""
            echo "Quick start:"
            echo "  1. balatro-mod-setup"
            echo "  2. balatro-update-wiki (first time only)"
            echo "  3. balatro-search-mods joker"
            echo "  4. balatro-install-from-wiki 'Mod Name'"
            echo ""
            echo "Tools available:"
            echo "  - Python ${pkgs.python3.version}, Lua ${pkgs.lua5_4.version}, Love2D"
            echo "  - Git, 7zip, file utilities"
            echo ""
            echo "Happy modding! 🃏"
          '';
        };
      });
}