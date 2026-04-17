use web_server::audio::util::get_file_path_root;

#[test]
fn test_get_file_path_root() {
    let base_path = "./voice_recordings/";
    let path_data = (12345i64, 67890i64, 2024i32, 5i32, "file".to_string());

    let result = get_file_path_root(base_path, &path_data);

    assert_eq!(result, "./voice_recordings/12345/67890/2024/5");
}

#[test]
fn test_get_file_path_root_different_base() {
    let base_path = "/tmp/recordings/";
    let path_data = (1i64, 2i64, 2023i32, 12i32, "test".to_string());

    let result = get_file_path_root(base_path, &path_data);

    assert_eq!(result, "/tmp/recordings/1/2/2023/12");
}
