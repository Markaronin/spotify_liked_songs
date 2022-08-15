use futures::stream::TryStreamExt;
use rspotify::{model::SavedTrack, prelude::*, scopes, AuthCodeSpotify, OAuth};
use serde::{Deserialize, Serialize};
use std::{env::current_dir, fs};

#[derive(Debug, Serialize)]
struct TrimmedTrackInfo {
    song_name: String,
    added_at: i64,
    artist_names: Vec<String>,
    album_name: String,
}
impl TrimmedTrackInfo {
    fn from_saved_track(saved_track: SavedTrack) -> Self {
        let mut val = TrimmedTrackInfo {
            song_name: saved_track.track.name,
            added_at: saved_track.added_at.timestamp(),
            artist_names: saved_track
                .track
                .artists
                .into_iter()
                .map(|artist| artist.name)
                .collect(),
            album_name: saved_track.track.album.name,
        };
        val.artist_names.sort();
        val
    }
}

#[derive(Deserialize)]
struct CredentialsFile {
    spotify_client_id: String,
    spotify_client_secret: String,
}
impl CredentialsFile {
    fn read() -> Self {
        serde_json::from_str(
            &fs::read_to_string(current_dir().unwrap().join("credentials.json")).unwrap(),
        )
        .unwrap()
    }
}

async fn get_liked_songs_list(creds: CredentialsFile) -> Vec<TrimmedTrackInfo> {
    use rspotify::Credentials;

    let creds = Credentials {
        id: creds.spotify_client_id,
        secret: Some(creds.spotify_client_secret),
    };

    let oauth = OAuth {
        redirect_uri: "http://localhost:8888/callback".to_string(),
        scopes: scopes!("user-library-read"),
        ..Default::default()
    };

    let spotify = {
        let mut temp_spotify = AuthCodeSpotify::new(creds, oauth);

        // Obtaining the access token
        let url = temp_spotify.get_authorize_url(false).unwrap();
        // This function requires the `cli` feature enabled.
        temp_spotify.prompt_for_token(&url).await.unwrap();
        temp_spotify
    };

    // Executing the futures concurrently
    let mut stream = spotify.current_user_saved_tracks(None);
    let mut liked_songs = Vec::new();
    while let Some(item) = stream.try_next().await.unwrap() {
        liked_songs.push(TrimmedTrackInfo::from_saved_track(item));
    }
    liked_songs.sort_by(|a, b| {
        a.added_at
            .cmp(&b.added_at)
            .then(a.song_name.cmp(&b.song_name))
    });
    liked_songs
}

async fn get_s3_client() -> aws_sdk_s3::Client {
    use aws_config::meta::region::RegionProviderChain;
    use aws_sdk_s3::Client;
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    let config = aws_config::from_env().region(region_provider).load().await;
    Client::new(&config)
}

async fn download_current_liked_songs() -> String {
    let resp = get_s3_client()
        .await
        .get_object()
        .bucket("markaronin-liked-songs")
        .key("liked-songs.txt")
        .send()
        .await
        .unwrap();
    let data = resp.body.collect().await;
    return String::from_utf8(data.unwrap().into_bytes().to_vec()).unwrap();
}

fn diff_liked_songs(new_liked_songs: &String, old_liked_songs: &String) {
    use diffy::{create_patch, PatchFormatter};

    let patch = create_patch(old_liked_songs, new_liked_songs);

    let f = PatchFormatter::new().with_color();

    print!("{}", f.fmt_patch(&patch));
}

async fn upload_liked_songs(new_liked_songs: String) {
    use aws_sdk_s3::types::ByteStream;
    let byte_stream = ByteStream::from(new_liked_songs.as_bytes().to_vec());
    get_s3_client()
        .await
        .put_object()
        .bucket("markaronin-liked-songs")
        .key("liked-songs.txt")
        .body(byte_stream)
        .send()
        .await
        .unwrap();
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let creds = CredentialsFile::read();

    let old_liked_songs = download_current_liked_songs().await;

    let new_liked_songs = get_liked_songs_list(creds)
        .await
        .into_iter()
        .map(|item| serde_json::to_string(&item).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    diff_liked_songs(&new_liked_songs, &old_liked_songs);

    upload_liked_songs(new_liked_songs).await;
}
