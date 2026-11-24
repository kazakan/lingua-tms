/// 새 엔티티 ID(UUIDv7 문자열). 시간순으로 정렬된다.
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
