use mfs_types::{IdentityContext, OwnerSpace};

#[test]
fn parses_account_scoped_owner_spaces() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");

    assert_eq!(ctx.account_id(), "acme");
    assert_eq!(ctx.user_space(), OwnerSpace::User("alice".into()));
    assert_eq!(
        ctx.agent_space(),
        OwnerSpace::Agent("alice__coding-agent".into())
    );
}
