from app.memory.memory_manager import MemoryManager


def test_memory_create_get_delete(tmp_path):
    database_url = f"sqlite:///{tmp_path / 'rei_test.sqlite3'}"
    manager = MemoryManager(database_url=database_url)

    created = manager.add_memory(
        key="user_name",
        value="Ярослав",
        source_message="меня зовут Ярослав",
        importance=3,
    )

    memories = manager.get_memories()
    assert len(memories) == 1
    assert memories[0].id == created.id
    assert memories[0].key == "user_name"
    assert memories[0].value == "Ярослав"

    assert manager.delete_memory(created.id) is True
    assert manager.get_memories() == []


def test_maybe_extract_memory_uses_explicit_markers(tmp_path):
    database_url = f"sqlite:///{tmp_path / 'rei_test.sqlite3'}"
    manager = MemoryManager(database_url=database_url)

    saved = manager.maybe_extract_memory("Запомни, что мой проект называется Rei.")

    assert len(saved) == 1
    assert saved[0].key == "note"
    assert "мой проект называется Rei" in saved[0].value


def test_maybe_extract_memory_skips_sensitive(tmp_path):
    database_url = f"sqlite:///{tmp_path / 'rei_test.sqlite3'}"
    manager = MemoryManager(database_url=database_url)

    saved = manager.maybe_extract_memory("Запомни мой пароль hunter2")

    assert saved == []
    assert manager.get_memories() == []

