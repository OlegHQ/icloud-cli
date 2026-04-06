use icloud_api::session::{load_session, SecretsBackend};
use icloud_api::HideMyEmailClient;
use icloud_api::Result as IResult;
use clap::Subcommand;

use crate::output::{hint, print_hme_action, print_hme_generate, print_json};
use crate::SessionArg;

#[derive(Subcommand)]
pub(crate) enum HmeCmd {
    /// List all Hide My Email aliases.
    List {
        #[command(flatten)]
        sess: SessionArg,
    },
    /// Generate a new random email address.
    Generate {
        #[command(flatten)]
        sess: SessionArg,
        /// Language code for the generated address.
        #[arg(long, default_value = "en-us")]
        lang: String,
    },
    /// Reserve (activate) a generated email address.
    Reserve {
        #[command(flatten)]
        sess: SessionArg,
        /// The generated email to reserve.
        email: String,
        /// Display label for this alias.
        #[arg(long)]
        label: Option<String>,
        /// Optional note.
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Deactivate an alias (can be reactivated later).
    Deactivate {
        #[command(flatten)]
        sess: SessionArg,
        /// Anonymous ID from `hme list`.
        anonymous_id: String,
    },
    /// Permanently delete an alias.
    Delete {
        #[command(flatten)]
        sess: SessionArg,
        /// Anonymous ID from `hme list`.
        anonymous_id: String,
    },
}

pub(crate) async fn handle_hme(json: bool, secrets: SecretsBackend, sub: HmeCmd) -> IResult<()> {
    match sub {
        HmeCmd::List { sess } => {
            let s = load_session(&sess.path(), secrets)?;
            let h = HideMyEmailClient::new(s)?;
            let v = h.list_aliases().await?;
            print_json(&v);
        }

        HmeCmd::Generate { sess, lang } => {
            let s = load_session(&sess.path(), secrets)?;
            let h = HideMyEmailClient::new(s)?;
            let email = h.generate(&lang).await?;
            print_hme_generate(json, email.as_deref());
            if !json {
                if let Some(e) = &email {
                    hint(&[&format!("icloud hme reserve {e} --label \"My Label\"  — activate it")]);
                }
            }
        }

        HmeCmd::Reserve { sess, email, label, note } => {
            let s = load_session(&sess.path(), secrets)?;
            let h = HideMyEmailClient::new(s)?;
            let lbl = label.unwrap_or_else(|| email.split('@').next().unwrap_or("").to_string());
            let ok = h.reserve(&email, &lbl, &note).await?;
            print_hme_action(json, &format!("Reserved {email}"), ok);
        }

        HmeCmd::Deactivate { sess, anonymous_id } => {
            let s = load_session(&sess.path(), secrets)?;
            let h = HideMyEmailClient::new(s)?;
            let ok = h.deactivate(&anonymous_id).await?;
            print_hme_action(json, "Deactivated", ok);
        }

        HmeCmd::Delete { sess, anonymous_id } => {
            let s = load_session(&sess.path(), secrets)?;
            let h = HideMyEmailClient::new(s)?;
            let ok = h.delete_alias(&anonymous_id).await?;
            print_hme_action(json, "Deleted", ok);
        }
    }
    Ok(())
}
