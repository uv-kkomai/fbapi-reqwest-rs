use crate::*;

impl Fbapi {
    pub async fn post_video(
        &self,
        access_token: &str,
        page_fbid: &str,
        url: &str,
        description: &str,
        thumb: Option<rusoto_core::ByteStream>,
        check_retry_count: usize,
        check_video_delay: usize,
        retry_count: usize,
        long_client: reqwest::Client,
        log: impl Fn(LogParams),
    ) -> Result<serde_json::Value, FbapiError> {
        let fbid = self
            .upload_video(
                access_token,
                page_fbid,
                url,
                description,
                thumb,
                check_retry_count,
                check_video_delay,
                retry_count,
                long_client,
                &log,
            )
            .await?;

        self.publish_video(access_token, &fbid, false, retry_count, &log)
            .await
    }

    /// Upload a video, check its status, and set thumbnail, returning the fbid without publishing.
    /// This allows you to handle the publish step separately using `publish_video`.
    pub async fn upload_video(
        &self,
        access_token: &str,
        page_fbid: &str,
        url: &str,
        description: &str,
        thumb: Option<rusoto_core::ByteStream>,
        check_retry_count: usize,
        check_video_delay: usize,
        retry_count: usize,
        long_client: reqwest::Client,
        log: impl Fn(LogParams),
    ) -> Result<String, FbapiError> {
        let fbid = video(
            &self.make_path(&format!("{}/videos", page_fbid)),
            access_token,
            url,
            description,
            &long_client,
            &log,
        )
        .await?;
        check_loop(
            &self.make_path(&format!(
                "{}?fields=status&access_token={}",
                fbid, access_token
            )),
            retry_count,
            check_retry_count,
            check_video_delay,
            &self.client,
            &log,
        )
        .await?;

        // サムネルがあれば、サムネル設定します。
        match thumb {
            Some(bytes) => {
                self.post_video_thumnail(access_token, &fbid, bytes, &log)
                    .await?;
            }
            None => {}
        };

        Ok(fbid)
    }

    /// Publish a video to newsfeed (and optionally to videos tab first).
    /// Set `via_videos_tab` to true to publish to videos tab before publishing to newsfeed.
    /// This allows you to handle errors during publishing while retaining the fbid.
    pub async fn publish_video(
        &self,
        access_token: &str,
        fbid: &str,
        via_videos_tab: bool,
        retry_count: usize,
        log: impl Fn(LogParams),
    ) -> Result<serde_json::Value, FbapiError> {
        if via_videos_tab {
            post_to_videos_tab(
                &self.make_path(&fbid),
                access_token,
                retry_count,
                &self.client,
                &log,
            )
            .await?;
        }

        post(
            &self.make_path(&fbid),
            access_token,
            retry_count,
            &self.client,
            &log,
        )
        .await
    }

    /// 直接 Newsfeed に投稿できない現象が発生している。
    /// 一度 VideosTab に公開してから Newsfeed に公開する。
    pub async fn post_video_via_videos_tab(
        &self,
        access_token: &str,
        page_fbid: &str,
        url: &str,
        description: &str,
        thumb: Option<rusoto_core::ByteStream>,
        check_retry_count: usize,
        check_video_delay: usize,
        retry_count: usize,
        long_client: reqwest::Client,
        log: impl Fn(LogParams),
    ) -> Result<serde_json::Value, FbapiError> {
        let fbid = self
            .upload_video(
                access_token,
                page_fbid,
                url,
                description,
                thumb,
                check_retry_count,
                check_video_delay,
                retry_count,
                long_client,
                &log,
            )
            .await?;

        self.publish_video(access_token, &fbid, true, retry_count, &log)
            .await
    }

    /// Publish a video to videos tab first, then to newsfeed.
    /// This is a convenience wrapper around `publish_video` with `via_videos_tab: true`.
    pub async fn publish_video_via_videos_tab(
        &self,
        access_token: &str,
        fbid: &str,
        retry_count: usize,
        log: impl Fn(LogParams),
    ) -> Result<serde_json::Value, FbapiError> {
        self.publish_video(access_token, fbid, true, retry_count, log)
            .await
    }
}

async fn video(
    path: &str,
    access_token: &str,
    file_url: &str,
    description: &str,
    long_client: &reqwest::Client,
    log: &impl Fn(LogParams),
) -> Result<String, FbapiError> {
    let params = vec![
        ("access_token", access_token),
        ("description", description),
        ("file_url", file_url),
        ("published", "true"),
        ("secret", "true"),
    ];
    let log_params = LogParams::new(&path, &params);
    let res: serde_json::Value = execute_retry(
        0,
        || async {
            long_client
                .post(path)
                .form(&params)
                .send()
                .await
                .map_err(|e| e.into())
        },
        log,
        log_params,
    )
    .await?;
    match res["id"].as_str() {
        Some(res) => Ok(res.to_owned()),
        None => {
            return Err(FbapiError::UnExpected(res));
        }
    }
}

async fn check(
    path: &str,
    retry_count: usize,
    client: &reqwest::Client,
    log: &impl Fn(LogParams),
) -> Result<String, FbapiError> {
    let log_params = LogParams::new(&path, &vec![]);
    let res = execute_retry(
        retry_count,
        || async { client.get(path).send().await.map_err(|e| e.into()) },
        log,
        log_params,
    )
    .await?;
    match res["status"]["video_status"].as_str() {
        Some(s) => Ok(s.to_owned()),
        None => return Err(FbapiError::UnExpected(res)),
    }
}

async fn post(
    path: &str,
    access_token: &str,
    retry_count: usize,
    client: &reqwest::Client,
    log: &impl Fn(LogParams),
) -> Result<serde_json::Value, FbapiError> {
    let params = vec![
        ("access_token", access_token),
        ("publish_to_news_feed", "true"),
        ("fields", "id"),
    ];
    let log_params = LogParams::new(path, &params);
    execute_retry(
        retry_count,
        || async {
            client
                .post(path)
                .form(&params)
                .send()
                .await
                .map_err(|e| e.into())
        },
        log,
        log_params,
    )
    .await
}

async fn post_to_videos_tab(
    path: &str,
    access_token: &str,
    retry_count: usize,
    client: &reqwest::Client,
    log: &impl Fn(LogParams),
) -> Result<serde_json::Value, FbapiError> {
    let params = vec![
        ("access_token", access_token),
        ("publish_to_videos_tab", "true"),
        ("fields", "id"),
    ];
    let log_params = LogParams::new(path, &params);
    execute_retry(
        retry_count,
        || async {
            client
                .post(path)
                .form(&params)
                .send()
                .await
                .map_err(|e| e.into())
        },
        log,
        log_params,
    )
    .await
}

#[derive(Debug, PartialEq, Eq)]
enum VideoStatusDecision {
    Ready,
    Continue,
    Failed,
}

fn decide_video_status(video_status: &str) -> VideoStatusDecision {
    match video_status {
        "ready" => VideoStatusDecision::Ready,
        "processing" | "uploading" => VideoStatusDecision::Continue,
        _ => VideoStatusDecision::Failed,
    }
}

async fn check_loop(
    path: &str,
    retry_count: usize,
    check_retry_count: usize,
    check_video_delay: usize,
    client: &reqwest::Client,
    log: &impl Fn(LogParams),
) -> Result<(), FbapiError> {
    for _ in 0..check_retry_count {
        let video_status = check(path, retry_count, client, log).await?;
        match decide_video_status(&video_status) {
            VideoStatusDecision::Ready => return Ok(()),
            VideoStatusDecision::Continue => {}
            VideoStatusDecision::Failed => return Err(FbapiError::VideoError),
        }
        sleep_sec(check_video_delay).await;
    }
    Err(FbapiError::VideoTimeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decide_video_status() {
        let cases = vec![
            ("ready", VideoStatusDecision::Ready),
            ("processing", VideoStatusDecision::Continue),
            ("uploading", VideoStatusDecision::Continue),
            ("error", VideoStatusDecision::Failed),
            ("expired", VideoStatusDecision::Failed),
            ("", VideoStatusDecision::Failed),
            ("READY", VideoStatusDecision::Failed),
            ("unknown_status", VideoStatusDecision::Failed),
        ];
        for (video_status, expected) in cases {
            assert_eq!(
                decide_video_status(video_status),
                expected,
                "video_status: {}",
                video_status
            );
        }
    }
}
