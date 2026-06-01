use anyhow::{Context, Result, bail, ensure};

use super::{
    BrowserRender, BrowserSession,
    forms::{
        default_form_submitter, form_control_is_reset_action, form_control_is_submit_action,
        form_control_submitter,
    },
};

impl BrowserSession {
    pub async fn submit_focused_form(&mut self) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot submit focused form: session has no current page");
        };
        let Some(focused) = self.entries[current_index].focused_control.clone() else {
            bail!("cannot submit focused form: no focused form control");
        };
        let (is_reset, submitter) = {
            let form = self.entries[current_index]
                .render
                .forms
                .get(focused.form_index)
                .with_context(|| {
                    format!(
                        "focused control form={} no longer exists",
                        focused.form_index
                    )
                })?;
            let control = form.controls.get(focused.control_index).with_context(|| {
                format!(
                    "focused control form={} control={} no longer exists",
                    focused.form_index, focused.control_index
                )
            })?;
            ensure!(
                control.name == focused.name && control.kind.eq_ignore_ascii_case(&focused.kind),
                "focused control {:?} is no longer the same form control",
                focused.name
            );
            let submitter = if form_control_is_submit_action(control) {
                form_control_submitter(control)
            } else {
                default_form_submitter(form)
            };
            (form_control_is_reset_action(control), submitter)
        };
        if is_reset {
            let render = render_current_entry_without_interaction(self, current_index)?;
            return self.reset_current_form_state_with_render(
                current_index,
                focused.form_index,
                render,
            );
        }
        self.submit_form_with_submitter(focused.form_index, &[], &submitter)
            .await
    }
}

fn render_current_entry_without_interaction(
    session: &mut BrowserSession,
    current_index: usize,
) -> Result<BrowserRender> {
    ensure!(
        current_index < session.entries.len(),
        "cannot render current entry: session entry {current_index} does not exist"
    );
    Ok(session.render_entry_page_state(current_index))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::browser::{BrowserRenderOptions, BrowserSession};

    #[tokio::test]
    async fn browser_session_required_text_blocks_submit_until_filled() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input id="q" name="q" required value="">
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        let error = session.submit_form(0, &[]).await.unwrap_err();

        assert!(error.to_string().contains("required field \"q\" is empty"));
        assert_eq!(session.current().unwrap().title, "Form");
        assert_eq!(session.snapshot().entries.len(), 1);

        session.set_form_field(0, "q", "rust browser").unwrap();
        let render = session.submit_form(0, &[]).await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?q=rust+browser"));
    }

    #[tokio::test]
    async fn browser_session_required_select_uses_selected_state() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <select name="kind" required>
                  <option value="">Pick one</option>
                  <option value="docs">Docs</option>
                </select>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();

        let error = session.submit_form(0, &[]).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("required field \"kind\" is empty")
        );

        session.select_form_option(0, 0, "docs").unwrap();
        assert_eq!(session.current().unwrap().text, "[Docs]");
        let render = session.submit_form(0, &[]).await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?kind=docs"));
    }

    #[tokio::test]
    async fn browser_session_required_checkbox_blocks_click_submit_until_checked() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input id="accept" type="checkbox" name="accept" required>
                <button id="go">Go</button>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();

        let error = session
            .click_selector_with_default_action("#go")
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("required field \"accept\" is not checked")
        );
        assert_eq!(session.current().unwrap().title, "Form");

        session
            .click_selector_with_default_action("#accept")
            .await
            .unwrap();
        let render = session
            .click_selector_with_default_action("#go")
            .await
            .unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?accept=on"));
    }

    #[tokio::test]
    async fn browser_session_novalidate_skips_required_validation() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get" novalidate>
                <input name="q" required value="">
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        let render = session.submit_form(0, &[]).await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?q="));
    }

    #[tokio::test]
    async fn browser_session_formnovalidate_submitter_skips_required_validation() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" required value="">
                <button id="skip" name="commit" value="skip" formnovalidate>Skip</button>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        let render = session
            .click_selector_with_default_action("#skip")
            .await
            .unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?q=&commit=skip"));
    }

    #[tokio::test]
    async fn browser_session_email_and_url_values_block_submit_until_valid() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="email" type="email" value="not-an-email">
                <input name="site" type="url" value="https://example.com">
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();

        let error = session.submit_form(0, &[]).await.unwrap_err();
        assert!(error.to_string().contains("invalid email value"));

        session
            .set_form_field(0, "email", "person@example.com")
            .unwrap();
        session.set_form_field(0, "site", "example.com").unwrap();
        let error = session.submit_form(0, &[]).await.unwrap_err();
        assert!(error.to_string().contains("invalid url value"));

        session
            .set_form_field(0, "site", "https://example.com/docs")
            .unwrap();
        let render = session.submit_form(0, &[]).await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with(
            "results.html?email=person%40example.com&site=https%3A%2F%2Fexample.com%2Fdocs"
        ));
    }

    #[tokio::test]
    async fn browser_session_focused_formnovalidate_submitter_skips_required_validation() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" required value="">
                <input id="skip" type="submit" name="commit" value="skip" formnovalidate>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        session.focus_selector("#skip").unwrap();
        let render = session.submit_focused_form().await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?q=&commit=skip"));
    }

    #[tokio::test]
    async fn browser_session_submitter_formaction_routes_click_submit() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        let alternate_page = dir.path().join("alternate.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="rust">
                <button id="alt" name="commit" value="alt" formaction="alternate.html">Alt</button>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>default</body></html>",
        )
        .unwrap();
        fs::write(
            &alternate_page,
            "<html><head><title>Alternate</title></head><body>alt</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        let render = session
            .click_selector_with_default_action("#alt")
            .await
            .unwrap();

        assert_eq!(render.title, "Alternate");
        assert!(render.source.ends_with("alternate.html?q=rust&commit=alt"));
    }

    #[tokio::test]
    async fn browser_session_submitter_formmethod_overrides_focused_submit() {
        let dir = tempfile::tempdir().unwrap();
        let form_page = dir.path().join("form.html");
        let results_page = dir.path().join("results.html");
        fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form action="post.html" method="post">
                <input name="q" value="rust">
                <input id="go" type="submit" name="commit" value="yes" formmethod=" get " formaction="results.html">
              </form>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            &results_page,
            "<html><head><title>Results</title></head><body>done</body></html>",
        )
        .unwrap();

        let mut session = BrowserSession::new(BrowserRenderOptions::default());
        session
            .navigate(&form_page.display().to_string())
            .await
            .unwrap();
        session.focus_selector("#go").unwrap();
        let render = session.submit_focused_form().await.unwrap();

        assert_eq!(render.title, "Results");
        assert!(render.source.ends_with("results.html?q=rust&commit=yes"));
    }
}
