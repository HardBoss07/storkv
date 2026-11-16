use storkv_proto::{is_unix_socket_addr, unix_socket_path};

#[test]
fn test_shared_unix_address_helpers() {
    assert!(is_unix_socket_addr("unix:/tmp/storkv.sock"));
    assert!(is_unix_socket_addr("/tmp/storkv.sock"));
    assert!(is_unix_socket_addr("relative/storkv.sock"));
    assert!(is_unix_socket_addr("C:\\temp\\storkv.sock"));
    assert!(!is_unix_socket_addr("127.0.0.1:7878"));
    assert!(!is_unix_socket_addr("localhost:7878"));
    assert_eq!(
        unix_socket_path("unix:/tmp/storkv.sock"),
        "/tmp/storkv.sock"
    );
    assert_eq!(unix_socket_path("/tmp/storkv.sock"), "/tmp/storkv.sock");
}
