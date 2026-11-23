//! Traduções: português (Brasil), inglês, espanhol e alemão.
//!
//! O idioma segue o da interface do Windows; se não for nenhum desses, usa
//! inglês. O texto em português é a chave: `tr("Configurações")` devolve a
//! versão no idioma atual. (Para testar: variável `TASKBEAT_LANG=en|es|de|pt`.)

use crate::config::Language;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Pt,
    En,
    Es,
    De,
}

/// Escolha manual do usuário (0 = automático).
static CHOSEN: AtomicU8 = AtomicU8::new(0);

/// Aplica a opção "Idioma" das configurações (vale na hora).
pub fn apply(l: Language) {
    let v = match l {
        Language::Auto => 0,
        Language::Pt => 1,
        Language::En => 2,
        Language::Es => 3,
        Language::De => 4,
    };
    CHOSEN.store(v, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CHOSEN.load(Ordering::Relaxed) {
        1 => return Lang::Pt,
        2 => return Lang::En,
        3 => return Lang::Es,
        4 => return Lang::De,
        _ => {}
    }
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        if let Ok(v) = std::env::var("TASKBEAT_LANG") {
            match v.to_ascii_lowercase().get(..2) {
                Some("pt") => return Lang::Pt,
                Some("es") => return Lang::Es,
                Some("de") => return Lang::De,
                Some("en") => return Lang::En,
                _ => {}
            }
        }
        // Idioma primário = 10 bits baixos do LANGID.
        match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
            0x16 => Lang::Pt,
            0x0a => Lang::Es,
            0x07 => Lang::De,
            _ => Lang::En,
        }
    })
}

/// Texto no idioma atual (a chave é o texto em português).
pub fn tr(pt: &'static str) -> &'static str {
    static MAP: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    let col = match lang() {
        Lang::Pt => return pt,
        Lang::En => 1,
        Lang::Es => 2,
        Lang::De => 3,
    };
    let map = MAP.get_or_init(|| T.iter().enumerate().map(|(i, r)| (r[0], i)).collect());
    match map.get(pt) {
        Some(&i) => T[i][col],
        None => pt,
    }
}

#[cfg(test)]
pub fn has(pt: &str) -> bool {
    T.iter().any(|r| r[0] == pt)
}

/// [português, inglês, espanhol, alemão]
static T: &[[&str; 4]] = &[
    // ---- geral / janela / bandeja
    ["Configurações", "Settings", "Configuración", "Einstellungen"],
    ["Configurações…", "Settings…", "Configuración…", "Einstellungen…"],
    ["Taskbeat — Configurações", "Taskbeat — Settings", "Taskbeat — Configuración", "Taskbeat — Einstellungen"],
    ["PRÉ-VISUALIZAÇÃO", "PREVIEW", "VISTA PREVIA", "VORSCHAU"],
    ["v1.0 · feito em Rust", "v1.0 · made in Rust", "v1.0 · hecho en Rust", "v1.0 · erstellt in Rust"],
    ["Ligado", "On", "Activado", "Ein"],
    ["Desligado", "Off", "Desactivado", "Aus"],
    ["Ativado", "On", "Activado", "Aktiviert"],
    ["Desativado", "Off", "Desactivado", "Deaktiviert"],
    ["A SEGUIR", "UP NEXT", "A CONTINUACIÓN", "ALS NÄCHSTES"],
    ["Caps Lock", "Caps Lock", "Bloq Mayús", "Feststelltaste"],
    ["Num Lock", "Num Lock", "Bloq Num", "Num-Taste"],
    ["Música na barra de tarefas", "Music on the taskbar", "Música en la barra de tareas", "Musik in der Taskleiste"],
    ["Barras de áudio dinâmicas", "Dynamic audio bars", "Barras de audio dinámicas", "Dynamische Audiobalken"],
    ["Cartão \"A seguir\"", "\"Up next\" card", "Tarjeta \"A continuación\"", "„Als Nächstes“-Karte"],
    ["Aviso de Caps Lock", "Caps Lock alert", "Aviso de Bloq Mayús", "Feststelltasten-Hinweis"],
    ["Sair do Taskbeat", "Quit Taskbeat", "Salir de Taskbeat", "Taskbeat beenden"],
    // ---- opções
    ["Arredondada", "Rounded", "Redondeada", "Abgerundet"],
    ["Arredondado", "Rounded", "Redondeado", "Abgerundet"],
    ["Círculo", "Circle", "Círculo", "Kreis"],
    ["Quadrada", "Square", "Cuadrada", "Quadratisch"],
    ["Quadrado", "Square", "Cuadrado", "Quadratisch"],
    ["Ao lado do relógio", "Next to the clock", "Junto al reloj", "Neben der Uhr"],
    ["À esquerda", "On the left", "A la izquierda", "Links"],
    ["Capa desfocada", "Blurred cover", "Portada difuminada", "Unscharfes Cover"],
    ["Cor do álbum", "Album color", "Color del álbum", "Albumfarbe"],
    ["Vidro", "Glass", "Cristal", "Glas"],
    ["Horizontal", "Horizontal", "Horizontal", "Horizontal"],
    ["Vertical", "Vertical", "Vertical", "Vertikal"],
    ["Automático", "Automatic", "Automático", "Automatisch"],
    ["Escuro", "Dark", "Oscuro", "Dunkel"],
    ["Claro", "Light", "Claro", "Hell"],
    ["Cor da capa", "Cover color", "Color de la portada", "Coverfarbe"],
    ["Cor do Windows", "Windows color", "Color de Windows", "Windows-Farbe"],
    ["Pequenos", "Small", "Pequeñas", "Klein"],
    ["Médios", "Medium", "Medianas", "Mittel"],
    ["Grandes", "Large", "Grandes", "Groß"],
    ["Embaixo", "Bottom", "Abajo", "Unten"],
    ["Centro", "Center", "Centro", "Mitte"],
    ["Topo", "Top", "Arriba", "Oben"],
    ["Acima do player", "Above the player", "Sobre el reproductor", "Über dem Player"],
    ["Iguais ao card", "Same as card", "Como la tarjeta", "Wie die Karte"],
    ["Pílula", "Pill", "Píldora", "Pille"],
    ["Retos", "Square", "Rectas", "Eckig"],
    ["Compacto", "Compact", "Compacto", "Kompakt"],
    ["Normal", "Normal", "Normal", "Normal"],
    ["Grande", "Large", "Grande", "Groß"],
    ["Barras", "Bars", "Barras", "Balken"],
    ["Espelhado", "Mirrored", "Reflejado", "Gespiegelt"],
    ["Onda", "Wave", "Onda", "Welle"],
    ["Cor de destaque do Windows", "Windows accent color", "Color de énfasis de Windows", "Windows-Akzentfarbe"],
    ["Arco-íris", "Rainbow", "Arcoíris", "Regenbogen"],
    ["Neon", "Neon", "Neón", "Neon"],
    ["Monocromático", "Monochrome", "Monocromático", "Monochrom"],
    // ---- páginas e seções
    ["Barra de tarefas", "Taskbar", "Barra de tareas", "Taskleiste"],
    ["O widget compacto que fica ao lado do relógio", "The compact widget next to the clock", "El widget compacto junto al reloj", "Das kompakte Widget neben der Uhr"],
    ["Card expandido", "Expanded card", "Tarjeta expandida", "Erweiterte Karte"],
    ["Abre ao clicar na música, com todos os controles", "Opens when you click the music, with all controls", "Se abre al hacer clic en la música, con todos los controles", "Öffnet sich per Klick auf die Musik, mit allen Steuerelementen"],
    ["A seguir", "Up next", "A continuación", "Als Nächstes"],
    ["Cartão com a próxima música quando a faixa termina", "Card with the next song when a track ends", "Tarjeta con la siguiente canción cuando termina la pista", "Karte mit dem nächsten Song, wenn ein Titel endet"],
    ["Aviso na tela ao ligar ou desligar Caps Lock e Num Lock", "On-screen alert when Caps Lock or Num Lock is turned on or off", "Aviso en pantalla al activar o desactivar Bloq Mayús y Bloq Num", "Bildschirmhinweis beim Ein- und Ausschalten von Feststell- und Num-Taste"],
    ["Visualizador", "Visualizer", "Visualizador", "Visualisierer"],
    ["Barras de áudio dinâmicas ao lado da música", "Dynamic audio bars next to the music", "Barras de audio dinámicas junto a la música", "Dynamische Audiobalken neben der Musik"],
    ["Geral", "General", "General", "Allgemein"],
    ["Inicialização, tela cheia e informações", "Startup, full screen and info", "Inicio, pantalla completa e información", "Autostart, Vollbild und Infos"],
    ["Conteúdo", "Content", "Contenido", "Inhalt"],
    ["Aparência", "Appearance", "Apariencia", "Darstellung"],
    ["Posição", "Position", "Posición", "Position"],
    ["Estilo", "Style", "Estilo", "Stil"],
    ["Comportamento", "Behavior", "Comportamiento", "Verhalten"],
    ["Avisos", "Alerts", "Avisos", "Hinweise"],
    ["Privacidade", "Privacy", "Privacidad", "Datenschutz"],
    ["Barras de áudio", "Audio bars", "Barras de audio", "Audiobalken"],
    ["Sistema", "System", "Sistema", "System"],
    ["Sobre", "About", "Acerca de", "Über"],
    // ---- barra de tarefas
    ["Mostrar música na barra", "Show music on the taskbar", "Mostrar música en la barra", "Musik in der Taskleiste anzeigen"],
    ["Capa, título e artista; clique para abrir o card", "Cover, title and artist; click to open the card", "Portada, título y artista; haz clic para abrir la tarjeta", "Cover, Titel und Künstler; Klick öffnet die Karte"],
    ["Botões de controle na barra", "Playback buttons on the taskbar", "Botones de control en la barra", "Steuertasten in der Taskleiste"],
    ["Anterior, play e próximo também na barra; por padrão ficam só no card", "Previous, play and next on the taskbar too; by default they're only in the card", "Anterior, reproducir y siguiente también en la barra; por defecto solo están en la tarjeta", "Zurück, Wiedergabe und Weiter auch in der Taskleiste; standardmäßig nur in der Karte"],
    ["Mostrar artista", "Show artist", "Mostrar artista", "Künstler anzeigen"],
    ["Segunda linha com o nome do artista", "Second line with the artist name", "Segunda línea con el nombre del artista", "Zweite Zeile mit dem Künstlernamen"],
    ["Indicador \"tocando agora\"", "\"Now playing\" indicator", "Indicador \"reproduciendo\"", "„Läuft gerade“-Anzeige"],
    ["Mini equalizador animado ao lado do título", "Small animated equalizer next to the title", "Mini ecualizador animado junto al título", "Kleiner animierter Equalizer neben dem Titel"],
    ["Barra de progresso", "Progress bar", "Barra de progreso", "Fortschrittsbalken"],
    ["Linha fina com o andamento da música", "Thin line showing the song's progress", "Línea fina con el avance de la canción", "Dünne Linie mit dem Songfortschritt"],
    ["Formato da capa", "Cover shape", "Forma de la portada", "Cover-Form"],
    ["Como a capa aparece na barra", "How the cover looks on the taskbar", "Cómo se ve la portada en la barra", "Wie das Cover in der Taskleiste aussieht"],
    ["Fundo em pílula", "Pill background", "Fondo de píldora", "Pillen-Hintergrund"],
    ["Destaque sutil sempre visível atrás do widget", "Subtle, always-visible highlight behind the widget", "Resalte sutil siempre visible detrás del widget", "Dezente, immer sichtbare Hervorhebung hinter dem Widget"],
    ["Rolar títulos longos", "Scroll long titles", "Desplazar títulos largos", "Lange Titel scrollen"],
    ["Letreiro suave quando o nome não cabe", "Smooth marquee when the name doesn't fit", "Marquesina suave cuando el nombre no cabe", "Sanftes Laufband, wenn der Name nicht passt"],
    ["Largura do título", "Title width", "Ancho del título", "Titelbreite"],
    ["Espaço para o nome da música e do artista", "Room for the song and artist names", "Espacio para la canción y el artista", "Platz für Song- und Künstlernamen"],
    ["Posição na barra", "Position on the taskbar", "Posición en la barra", "Position in der Taskleiste"],
    ["Onde o widget fica na barra de tarefas", "Where the widget sits on the taskbar", "Dónde se ubica el widget en la barra de tareas", "Wo das Widget in der Taskleiste sitzt"],
    ["Ajuste horizontal", "Horizontal offset", "Ajuste horizontal", "Horizontaler Versatz"],
    ["Desloca o widget para a esquerda ou para a direita", "Moves the widget left or right", "Mueve el widget a la izquierda o a la derecha", "Verschiebt das Widget nach links oder rechts"],
    // ---- card
    ["Fundo", "Background", "Fondo", "Hintergrund"],
    ["Capa desfocada, cor extraída da capa ou vidro neutro", "Blurred cover, a color taken from the cover, or neutral glass", "Portada difuminada, color de la portada o cristal neutro", "Unscharfes Cover, Farbe aus dem Cover oder neutrales Glas"],
    ["Layout", "Layout", "Diseño", "Layout"],
    ["Capa ao lado ou capa grande em cima", "Cover on the side or a large cover on top", "Portada al lado o portada grande arriba", "Cover daneben oder großes Cover oben"],
    ["Tema", "Theme", "Tema", "Design"],
    ["Segue o Windows ou fixa claro/escuro", "Follows Windows or stays light/dark", "Sigue a Windows o fija claro/oscuro", "Folgt Windows oder bleibt hell/dunkel"],
    ["Cor de destaque", "Accent color", "Color de énfasis", "Akzentfarbe"],
    ["Botões e progresso na cor da capa ou do Windows", "Buttons and progress in the cover's or Windows' color", "Botones y progreso con el color de la portada o de Windows", "Tasten und Fortschritt in Cover- oder Windows-Farbe"],
    ["Cantos", "Corners", "Esquinas", "Ecken"],
    ["Raio das bordas do card", "Card corner radius", "Radio de las esquinas de la tarjeta", "Eckenradius der Karte"],
    ["Botão play", "Play button", "Botón de reproducir", "Wiedergabetaste"],
    ["Formato do botão principal", "Shape of the main button", "Forma del botón principal", "Form der Haupttaste"],
    ["Tempo decorrido e restante; arraste para pular", "Elapsed and remaining time; drag to seek", "Tiempo transcurrido y restante; arrastra para saltar", "Verstrichene und verbleibende Zeit; ziehen zum Springen"],
    ["Volume", "Volume", "Volumen", "Lautstärke"],
    ["Slider de volume e botão de mudo", "Volume slider and mute button", "Control de volumen y botón de silencio", "Lautstärkeregler und Stummschalter"],
    ["Mostrar app de origem", "Show source app", "Mostrar app de origen", "Quell-App anzeigen"],
    ["Nome do app (Spotify, Brave…) no card e ícone sobre a capa", "App name (Spotify, Brave…) on the card and an icon over the cover", "Nombre de la app (Spotify, Brave…) en la tarjeta e icono sobre la portada", "App-Name (Spotify, Brave…) auf der Karte und Symbol auf dem Cover"],
    ["Fechar sozinho após", "Close automatically after", "Cerrar automáticamente tras", "Automatisch schließen nach"],
    ["Tempo sem o mouse em cima do card", "Time without the mouse over the card", "Tiempo sin el ratón sobre la tarjeta", "Zeit ohne Maus über der Karte"],
    ["Aleatório e repetir", "Shuffle and repeat", "Aleatorio y repetir", "Zufall und Wiederholen"],
    ["Aparecem quando o player permite (Spotify, Media Player). Navegadores como Brave e Chrome não permitem", "Shown when the player allows it (Spotify, Media Player). Browsers like Brave and Chrome don't", "Aparecen cuando el reproductor lo permite (Spotify, Media Player). Navegadores como Brave y Chrome no lo permiten", "Erscheinen, wenn der Player es erlaubt (Spotify, Media Player). Browser wie Brave und Chrome erlauben es nicht"],
    // ---- a seguir
    ["Mostrar \"A seguir\"", "Show \"Up next\"", "Mostrar \"A continuación\"", "„Als Nächstes“ anzeigen"],
    ["Só quando o player troca de faixa sozinho; trocas manuais não mostram", "Only when the player changes tracks on its own; manual skips don't show it", "Solo cuando el reproductor cambia de pista solo; los cambios manuales no lo muestran", "Nur wenn der Player selbst weiterschaltet; manuelles Überspringen zeigt sie nicht"],
    ["Duração", "Duration", "Duración", "Dauer"],
    ["Quanto tempo o cartão fica na tela", "How long the card stays on screen", "Cuánto tiempo queda la tarjeta en pantalla", "Wie lange die Karte sichtbar bleibt"],
    // ---- caps lock
    ["Mostra quando as maiúsculas forem ativadas ou desativadas", "Shows when capital letters are turned on or off", "Muestra cuando se activan o desactivan las mayúsculas", "Zeigt an, wenn die Großschreibung ein- oder ausgeschaltet wird"],
    ["Mostra quando o teclado numérico mudar", "Shows when the number pad changes", "Muestra cuando cambia el teclado numérico", "Zeigt an, wenn sich der Ziffernblock ändert"],
    ["Avisar também ao desligar", "Also alert when turned off", "Avisar también al desactivar", "Auch beim Ausschalten anzeigen"],
    ["Desligado: só aparece quando a tecla é ativada", "Off: only appears when the key is turned on", "Desactivado: solo aparece al activar la tecla", "Aus: erscheint nur beim Einschalten"],
    ["Testar", "Test", "Probar", "Testen"],
    ["Mostra o aviso agora, na posição escolhida", "Shows the alert now, in the chosen position", "Muestra el aviso ahora, en la posición elegida", "Zeigt den Hinweis jetzt an der gewählten Position"],
    ["Mostrar agora", "Show now", "Mostrar ahora", "Jetzt anzeigen"],
    ["Onde aparece", "Where it appears", "Dónde aparece", "Wo er erscheint"],
    ["Predefinições rápidas; ajuste fino logo abaixo", "Quick presets; fine-tune below", "Ajustes rápidos; ajuste fino debajo", "Schnellvorgaben; Feinjustierung darunter"],
    ["Move o aviso para a esquerda ou para a direita", "Moves the alert left or right", "Mueve el aviso a la izquierda o a la derecha", "Verschiebt den Hinweis nach links oder rechts"],
    ["Ajuste vertical", "Vertical offset", "Ajuste vertical", "Vertikaler Versatz"],
    ["Move o aviso para cima ou para baixo", "Moves the alert up or down", "Mueve el aviso hacia arriba o hacia abajo", "Verschiebt den Hinweis nach oben oder unten"],
    ["Formato do aviso e do ícone", "Shape of the alert and its icon", "Forma del aviso y del icono", "Form von Hinweis und Symbol"],
    ["Tamanho", "Size", "Tamaño", "Größe"],
    ["Tamanho do aviso e do texto", "Size of the alert and its text", "Tamaño del aviso y del texto", "Größe von Hinweis und Text"],
    ["Interruptor", "Switch", "Interruptor", "Schalter"],
    ["Chave animada no lado direito", "Animated switch on the right", "Interruptor animado a la derecha", "Animierter Schalter rechts"],
    ["Quanto tempo o aviso fica na tela", "How long the alert stays on screen", "Cuánto tiempo queda el aviso en pantalla", "Wie lange der Hinweis sichtbar bleibt"],
    ["Só essas duas teclas são observadas; nada do que você digita é lido ou guardado", "Only these two keys are watched; nothing you type is read or stored", "Solo se observan estas dos teclas; nada de lo que escribes se lee ni se guarda", "Nur diese zwei Tasten werden beobachtet; nichts, was du tippst, wird gelesen oder gespeichert"],
    // ---- visualizador
    ["Mostrar barras dinâmicas", "Show dynamic bars", "Mostrar barras dinámicas", "Dynamische Balken anzeigen"],
    ["Desligado por padrão; quando ligado, captura o som do PC", "Off by default; when on, it captures the PC's sound", "Desactivado por defecto; al activarlo, captura el sonido del PC", "Standardmäßig aus; wenn an, wird der PC-Ton erfasst"],
    ["Formato das barras", "Bar shape", "Forma de las barras", "Balkenform"],
    ["Cores", "Colors", "Colores", "Farben"],
    ["Cor da capa, do Windows ou paletas", "Cover color, Windows color or palettes", "Color de la portada, de Windows o paletas", "Cover-, Windows-Farbe oder Paletten"],
    ["Quantidade de barras", "Number of bars", "Cantidad de barras", "Anzahl der Balken"],
    ["Mais barras = mais detalhe", "More bars = more detail", "Más barras = más detalle", "Mehr Balken = mehr Details"],
    ["Largura", "Width", "Ancho", "Breite"],
    ["Tamanho do visualizador", "Visualizer size", "Tamaño del visualizador", "Größe des Visualisierers"],
    ["Sensibilidade", "Sensitivity", "Sensibilidad", "Empfindlichkeit"],
    ["Ganho extra aplicado ao espectro", "Extra gain applied to the spectrum", "Ganancia extra aplicada al espectro", "Zusätzliche Verstärkung des Spektrums"],
    ["Picos", "Peaks", "Picos", "Spitzen"],
    ["Marcadores que caem devagar", "Markers that fall slowly", "Marcadores que caen despacio", "Langsam fallende Markierungen"],
    ["Só quando um player estiver tocando", "Only while a player is playing", "Solo cuando un reproductor esté sonando", "Nur während ein Player spielt"],
    ["Ignora jogos, chamadas e notificações", "Ignores games, calls and notifications", "Ignora juegos, llamadas y notificaciones", "Ignoriert Spiele, Anrufe und Benachrichtigungen"],
    // ---- geral
    ["Iniciar com o Windows", "Start with Windows", "Iniciar con Windows", "Mit Windows starten"],
    ["Abre automaticamente quando você entra no Windows", "Opens automatically when you sign in to Windows", "Se abre automáticamente al iniciar sesión en Windows", "Öffnet automatisch bei der Windows-Anmeldung"],
    ["Ocultar em tela cheia", "Hide in full screen", "Ocultar en pantalla completa", "Im Vollbild ausblenden"],
    ["Some durante jogos, vídeos e apresentações", "Hides during games, videos and presentations", "Se oculta en juegos, vídeos y presentaciones", "Blendet sich bei Spielen, Videos und Präsentationen aus"],
    ["Restaurar padrões", "Restore defaults", "Restaurar valores predeterminados", "Standardwerte wiederherstellen"],
    ["Volta todas as opções para os valores originais", "Resets every option to its original value", "Devuelve todas las opciones a sus valores originales", "Setzt alle Optionen auf die Ausgangswerte zurück"],
    ["Restaurar", "Restore", "Restaurar", "Zurücksetzen"],
    ["Taskbeat", "Taskbeat", "Taskbeat", "Taskbeat"],
    ["Idioma", "Language", "Idioma", "Sprache"],
    // ---- instalador
    ["Instalar o Taskbeat", "Install Taskbeat", "Instalar Taskbeat", "Taskbeat installieren"],
    ["Remover o Taskbeat", "Remove Taskbeat", "Quitar Taskbeat", "Taskbeat entfernen"],
    ["Widget de música para a barra de tarefas", "Music widget for the taskbar", "Widget de música para la barra de tareas", "Musik-Widget für die Taskleiste"],
    ["Pasta de instalação", "Install folder", "Carpeta de instalación", "Installationsordner"],
    ["Alterar…", "Change…", "Cambiar…", "Ändern…"],
    ["Escolha onde instalar o Taskbeat", "Choose where to install Taskbeat", "Elige dónde instalar Taskbeat", "Wähle, wo Taskbeat installiert wird"],
    ["Atalho na área de trabalho", "Desktop shortcut", "Acceso directo en el escritorio", "Desktop-Verknüpfung"],
    ["Abrir o Taskbeat ao terminar", "Open Taskbeat when done", "Abrir Taskbeat al terminar", "Taskbeat danach öffnen"],
    ["Instala só para você; não precisa de administrador.", "Installs just for you; no administrator needed.", "Se instala solo para ti; no necesita administrador.", "Wird nur für dich installiert; keine Administratorrechte nötig."],
    ["Cancelar", "Cancel", "Cancelar", "Abbrechen"],
    ["Instalar", "Install", "Instalar", "Installieren"],
    ["Atualizar", "Update", "Actualizar", "Aktualisieren"],
    ["Remover", "Remove", "Quitar", "Entfernen"],
    ["Instalando…", "Installing…", "Instalando…", "Wird installiert…"],
    ["Removendo…", "Removing…", "Quitando…", "Wird entfernt…"],
    ["Concluir", "Finish", "Finalizar", "Fertig"],
    ["Fechar", "Close", "Cerrar", "Schließen"],
    ["Pronto! O Taskbeat foi instalado.", "Done! Taskbeat is installed.", "¡Listo! Taskbeat está instalado.", "Fertig! Taskbeat ist installiert."],
    ["A música aparece na barra de tarefas, ao lado do relógio, sempre que algo estiver tocando.", "The music shows up on the taskbar, next to the clock, whenever something is playing.", "La música aparece en la barra de tareas, junto al reloj, siempre que algo esté sonando.", "Die Musik erscheint in der Taskleiste neben der Uhr, sobald etwas läuft."],
    ["As configurações ficam no ícone do Taskbeat perto do relógio e no Menu Iniciar.", "Settings are in the Taskbeat icon near the clock and in the Start menu.", "La configuración está en el icono de Taskbeat junto al reloj y en el menú Inicio.", "Die Einstellungen findest du über das Taskbeat-Symbol neben der Uhr und im Startmenü."],
    ["Não foi possível instalar", "Couldn't install", "No se pudo instalar", "Installation fehlgeschlagen"],
    ["Não foi possível gravar nessa pasta. Escolha outra, por exemplo dentro da sua pasta de usuário.", "Can't write to that folder. Choose another one, for example inside your user folder.", "No se puede escribir en esa carpeta. Elige otra, por ejemplo dentro de tu carpeta de usuario.", "In diesen Ordner kann nicht geschrieben werden. Wähle einen anderen, z. B. in deinem Benutzerordner."],
    ["Remover o Taskbeat?", "Remove Taskbeat?", "¿Quitar Taskbeat?", "Taskbeat entfernen?"],
    ["O app, os atalhos e a inicialização com o Windows serão removidos.", "The app, its shortcuts and the startup entry will be removed.", "Se quitarán la app, los accesos directos y el inicio con Windows.", "Die App, ihre Verknüpfungen und der Autostart werden entfernt."],
    ["Apagar também minhas configurações", "Also delete my settings", "Borrar también mi configuración", "Auch meine Einstellungen löschen"],
    ["O Taskbeat foi removido.", "Taskbeat was removed.", "Taskbeat se ha quitado.", "Taskbeat wurde entfernt."],
    ["Obrigado por usar!", "Thanks for using it!", "¡Gracias por usarlo!", "Danke fürs Benutzen!"],
    ["Automático segue o idioma do Windows (outros idiomas usam inglês)", "Automatic follows the Windows language (other languages use English)", "Automático sigue el idioma de Windows (otros idiomas usan inglés)", "Automatisch folgt der Windows-Sprache (andere Sprachen nutzen Englisch)"],
    ["Automático (idioma do Windows)", "Automatic (Windows language)", "Automático (idioma de Windows)", "Automatisch (Windows-Sprache)"],
    ["Português (Brasil)", "Português (Brasil)", "Português (Brasil)", "Português (Brasil)"],
    ["English", "English", "English", "English"],
    ["Español", "Español", "Español", "Español"],
    ["Deutsch", "Deutsch", "Deutsch", "Deutsch"],
    ["Feito em Rust · WASAPI loopback · System Media Transport Controls", "Made in Rust · WASAPI loopback · System Media Transport Controls", "Hecho en Rust · WASAPI loopback · System Media Transport Controls", "Erstellt in Rust · WASAPI loopback · System Media Transport Controls"],
];
