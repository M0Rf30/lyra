app-title = Lyra
about = About
repository = Repository
file = File
view = View
switch-to-grid = Grid view
switch-to-list = List view
albums = Albums
artists = Artists
songs = Songs
equalizer = Equalizer
lyrics = Lyrics
quit = Quit
add-music-folder = Add Music Folder
scan-library = Scan Library
scanning-library = Scanning library...
no-albums = No albums found
no-artists = No artists found
no-songs = No songs found
play = Play
pause = Pause
next = Next
previous = Previous
shuffle = Shuffle
repeat = Repeat
volume = Volume
search = Search
settings = Settings
providers = Providers
add-mpd-server = Add MPD Server
mpd-name = Name
mpd-host = Host
mpd-port = Port
mpd-password = Password
save = Save
remove = Remove
test-connection = Test Connection
connected = Connected
connection-failed = Connection Failed
add-subsonic-server = Add Subsonic Server
subsonic-name = Name
subsonic-url = Server URL
subsonic-username = Username
subsonic-password = Password
subsonic-accept-invalid-certs = Accept invalid TLS certificates
no-music-dirs = No music directories configured
no-providers = No remote providers configured
playlists = Playlists
genres = Genres
no-playlists = No playlists
no-genres = No genres found
playlist-tracks = { $count } tracks
playlist-empty = This playlist is empty
create-playlist = Create Playlist
delete-playlist = Delete
rename-playlist = Rename
play-all = Play All
crossfade = Crossfade
crossfade-duration = Crossfade Duration
crossfade-seconds = { $secs }s
crossfade-disabled = Disabled
replay-gain = Replay Gain
replay-gain-off = Off
replay-gain-track = Track
replay-gain-album = Album
replay-gain-auto = Auto
transcoding = Transcoding
transcoding-bitrate = Max Bitrate
transcoding-format = Format
transcoding-original = Original
transcoding-bandwidth-estimate = Estimated bandwidth savings: ~{ $percent }%

# SettingsView
settings-library = Library
settings-playback = Playback
settings-shortcuts = Shortcuts
settings-about = About

# RowFoundation
favorite-add = Add to favorites
favorite-remove = Remove from favorites

# CompactBar
no-track-playing = No track playing
show-now-playing = Show now playing

# EqualizerView
equalizer-enabled = Equalizer Enabled
equalizer-section-preset = Preset
equalizer-save-preset-as = Save As
equalizer-preset-name-placeholder = Preset name...
equalizer-delete-preset = Delete
equalizer-reset-preset = Reset
equalizer-section-autoeq = AutoEQ
equalizer-autoeq-search-placeholder = Search headphones...
equalizer-autoeq-no-matches = No matches
equalizer-autoeq-too-many-matches = 50+ matches — refine your search
equalizer-autoeq-match-count = { $count } matches
equalizer-autoeq-profile-count = { $count } profiles
equalizer-autoeq-search-hint = Type 2+ chars to search
equalizer-autoeq-profiles-loaded = { $count } profiles loaded
equalizer-autoeq-loading = Loading...
equalizer-autoeq-load-profiles = Load AutoEQ Profiles
equalizer-section-preamp = Preamp
equalizer-preamp-label = Preamp:
equalizer-preamp-value = { $db } dB
equalizer-section-bands = Bands

# LyricsView
lyrics-loading = Loading lyrics...
lyrics-unavailable = No lyrics available
lyrics-search-online = Search Online

# AlbumsView
albums-empty-hint = Add music directories in Settings to get started
play-album = Play Album
album-tooltip = { $title } — { $artist }
back-to-albums = Back to albums

# ExpandedView
expanded-collapse = Back to library
expanded-empty-hint = Choose a song from your library to start listening

# SmallViews
artists-empty-hint = Artists will appear here once your library is scanned
back-to-artists = Back to artists
artist-album-count-one = { $count } album
artist-album-count-other = { $count } albums
artist-track-count-one = { $count } track
artist-track-count-other = { $count } tracks
genres-empty-hint = Genres will appear once your library is scanned.
no-tracks-found = No tracks found
genre-empty-hint = No tracks found for this genre.
genre-track-count-one = { $count } track
genre-track-count-other = { $count } tracks
new-playlist-placeholder = New playlist name...
playlist-name-placeholder = Playlist name...
playlists-empty-hint = Create a playlist to organize your music
delete-playlist-tooltip = Delete playlist
back-to-playlists = Back to playlists
remove-from-playlist = Remove from playlist
playlist-empty-hint = Add tracks from the Songs view.
playlist-track-count-one = { $count } track
playlist-track-count-other = { $count } tracks

# AppShell
search-library = Search library
toast-provider-connect-failed = Failed to connect to { $provider }: { $reason }
toast-open-files-failed = No playable audio files found

# SongsTable
songs-column-number = #
songs-column-title = Title
songs-column-artist = Artist
songs-column-album = Album
songs-column-duration = Duration
songs-add-to-playlist = Add to "{ $playlist }"
songs-favorites-filter = Favorites
songs-empty-hint = Scan your library from File > Rescan
songs-no-matches = No matching tracks
songs-no-matches-hint = Try clearing the favorites or genre filter

# PodcastsView
podcasts = Podcasts
podcast-search-placeholder = Search podcast directory...
searching = Searching...
subscribe = Subscribe
podcast-url-placeholder = Podcast feed URL...
refresh-all = Refresh All
subscriptions = Subscriptions
no-podcasts = No podcasts subscribed
podcasts-empty-hint = Search the directory above or paste a feed URL to subscribe
refresh-podcast-tooltip = Refresh feed
unsubscribe-tooltip = Unsubscribe
back-to-podcasts = Back to podcasts
resume-at = Resume at { $position }
mark-played-tooltip = Toggle played
download-episode-tooltip = Download for offline playback
delete-download-tooltip = Delete download
no-episodes = No episodes found
no-episodes-hint = This podcast's feed has no episodes yet
toast-podcast-search-failed = Podcast search failed: { $reason }
toast-podcast-subscribe-failed = Failed to subscribe: { $reason }
toast-podcast-refresh-failed = Failed to refresh podcast: { $reason }
toast-episode-download-failed = Failed to download episode: { $reason }

# RadioView
radio = Radio
radio-search-placeholder = Search radio stations...
radio-discover = Discover popular stations
play-station-tooltip = Play station
add-station = Add
station-name-placeholder = Station name...
station-url-placeholder = Stream URL...
my-stations = My Stations
no-stations = No stations saved
stations-empty-hint = Search the directory above or paste a stream URL to add one
remove-station-tooltip = Remove station
toast-radio-search-failed = Radio search failed: { $reason }
toast-radio-play-failed = Failed to play station: { $reason }

# ConvertView
convert = Convert
convert-add-files = Add Files
convert-output-dir = Output Directory
convert-choose-dir = Choose…
convert-format = Format
convert-sample-rate = Sample Rate
convert-rate-source = Source
convert-start = Start
convert-clear-finished = Clear Finished
no-convert-jobs = No conversion jobs
convert-empty-hint = Add audio or video files, or a .cue sheet, to convert or rip them
convert-kind-convert = Convert
convert-kind-cuesplit = Split CUE
convert-state-queued = Queued
convert-state-running = Converting…
convert-state-done = Done
convert-state-failed = Failed: { $error }
convert-state-cancelled = Cancelled
convert-cancel-tooltip = Cancel
convert-format-flac = FLAC
convert-format-wav16 = WAV (16-bit)
convert-format-wav24 = WAV (24-bit)
convert-format-wav32float = WAV (32-bit float)

# Visualizer
viz-toggle-presets = Browse presets
viz-presets = Presets
viz-preset-search = Search presets...
viz-preset-empty = No presets match your search
viz-close-presets = Close
viz-next-preset = Next preset
viz-lock = Lock preset
viz-beat-sensitivity = Beat sensitivity

# --- folders ---
folders = Folders
no-folders = No music found
folders-empty-hint = Add music directories in Settings to get started
folders-root = Library
folders-up = Up one level
folder-empty = Empty folder
folder-empty-hint = This folder has no tracks or subfolders.
play-folder = Play Folder
queue-folder-tooltip = Queue folder
folder-track-count-one = { $count } track
folder-track-count-other = { $count } tracks
queued-tracks = Added { $count } tracks to the queue

# --- artist tags ---
split-artist-tags = Split multi-artist tags
split-artist-tags-description = Split tags like "Artist A feat. B" into individual artists so each gets its own entry and collaborations are found under any contributor.
artist-tag-delimiters = Delimiters
artist-tag-delimiters-description = Separators used to split artist tags, tried longest-first. Edit as a " | "-separated list.
artist-tag-delimiters-placeholder = ; | feat. | & | / | ...
reset-to-defaults = Reset to defaults

# --- smart playlists ---
smart-playlists = Smart Playlists
new-smart-playlist = New Smart Playlist
edit-smart-playlist = Edit Smart Playlist
no-smart-playlists = No smart playlists
smart-playlists-empty-hint = Create a rule-based playlist that updates automatically as your library changes
play-smart-playlist-tooltip = Play
edit-smart-playlist-tooltip = Edit rules
delete-smart-playlist-tooltip = Delete smart playlist
back-to-smart-playlists = Back to smart playlists
smart-playlist-track-count-one = { $count } track
smart-playlist-track-count-other = { $count } tracks
smart-playlist-empty = No tracks match these rules
smart-playlist-empty-hint = Loosen a rule, or add tracks to your library
smart-playlist-name-placeholder = Smart playlist name...
smart-playlist-match = Match
smart-playlist-match-all = All rules
smart-playlist-match-any = Any rule
smart-playlist-rules-heading = Rules
add-rule = Add Rule
remove-rule-tooltip = Remove rule
smart-playlist-value-placeholder = Value...
smart-playlist-value2-placeholder = and...
smart-playlist-order-by = Order by
smart-playlist-order-desc = Descending
smart-playlist-limit = Limit to
smart-playlist-limit-placeholder = tracks
smart-playlist-cancel-edit = Cancel
smart-playlist-field-title = Title
smart-playlist-field-artist = Artist
smart-playlist-field-album-artist = Album Artist
smart-playlist-field-album = Album
smart-playlist-field-genre = Genre
smart-playlist-field-year = Year
smart-playlist-field-rating = Rating
smart-playlist-field-favorite = Favorite
smart-playlist-field-duration = Duration (seconds)
smart-playlist-field-bitrate = Bitrate
smart-playlist-field-sample-rate = Sample Rate
smart-playlist-op-is = is
smart-playlist-op-is-not = is not
smart-playlist-op-contains = contains
smart-playlist-op-not-contains = does not contain
smart-playlist-op-starts-with = starts with
smart-playlist-op-ends-with = ends with
smart-playlist-op-greater-than = is greater than
smart-playlist-op-less-than = is less than
smart-playlist-op-between = is between
smart-playlist-order-title = Title
smart-playlist-order-artist = Artist
smart-playlist-order-album = Album
smart-playlist-order-year = Year
smart-playlist-order-rating = Rating
smart-playlist-order-duration = Duration
smart-playlist-order-random = Random
smart-playlist-order-recently-added = Recently Added
