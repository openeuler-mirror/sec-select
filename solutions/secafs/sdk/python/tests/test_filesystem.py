"""Filesystem Integration Tests"""

import pytest

from secafs_sdk import ErrnoException, Filesystem


@pytest.mark.asyncio
class TestFilesystemWriteOperations:
    """Filesystem write operations"""

    async def test_write_and_read_simple_text_file(self, db):
        """Should write and read a simple text file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/test.txt", "Hello, World!")
        content = await fs.read_file("/test.txt")
        assert content == "Hello, World!"

    async def test_write_files_in_subdirectories(self, db):
        """Should write and read files in subdirectories"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/subdir/file.txt", "nested content")
        content = await fs.read_file("/dir/subdir/file.txt")
        assert content == "nested content"

    async def test_overwrite_existing_file(self, db):
        """Should overwrite existing file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/overwrite.txt", "original content")
        await fs.write_file("/overwrite.txt", "new content")
        content = await fs.read_file("/overwrite.txt")
        assert content == "new content"

    async def test_handle_empty_file_content(self, db):
        """Should handle empty file content"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/empty.txt", "")
        content = await fs.read_file("/empty.txt")
        assert content == ""

    async def test_handle_large_file_content(self, db):
        """Should handle large file content"""
        fs = await Filesystem.from_database(db)

        large_content = "x" * 100000
        await fs.write_file("/large.txt", large_content)
        content = await fs.read_file("/large.txt")
        assert content == large_content

    async def test_special_characters_in_content(self, db):
        """Should handle files with special characters in content"""
        fs = await Filesystem.from_database(db)

        special_content = "Special chars: \n\t\r\"'\\"
        await fs.write_file("/special.txt", special_content)
        content = await fs.read_file("/special.txt")
        assert content == special_content


@pytest.mark.asyncio
class TestFilesystemReadOperations:
    """Filesystem read operations"""

    async def test_error_reading_nonexistent_file(self, db):
        """Should throw error when reading non-existent file"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException):
            await fs.read_file("/non-existent.txt")

    async def test_read_multiple_files(self, db):
        """Should read multiple different files"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/file1.txt", "content 1")
        await fs.write_file("/file2.txt", "content 2")
        await fs.write_file("/file3.txt", "content 3")

        assert await fs.read_file("/file1.txt") == "content 1"
        assert await fs.read_file("/file2.txt") == "content 2"
        assert await fs.read_file("/file3.txt") == "content 3"


@pytest.mark.asyncio
class TestFilesystemDirectoryOperations:
    """Filesystem directory operations"""

    async def test_list_files_in_root(self, db):
        """Should list files in root directory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/file1.txt", "content 1")
        await fs.write_file("/file2.txt", "content 2")
        await fs.write_file("/file3.txt", "content 3")

        files = await fs.readdir("/")
        assert "file1.txt" in files
        assert "file2.txt" in files
        assert "file3.txt" in files
        assert len(files) == 3

    async def test_list_files_in_subdirectory(self, db):
        """Should list files in subdirectory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file1.txt", "content 1")
        await fs.write_file("/dir/file2.txt", "content 2")
        await fs.write_file("/other/file3.txt", "content 3")

        files = await fs.readdir("/dir")
        assert "file1.txt" in files
        assert "file2.txt" in files
        assert "file3.txt" not in files
        assert len(files) == 2

    async def test_distinguish_files_in_different_directories(self, db):
        """Should distinguish between files in different directories"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir1/file.txt", "content 1")
        await fs.write_file("/dir2/file.txt", "content 2")

        files1 = await fs.readdir("/dir1")
        files2 = await fs.readdir("/dir2")

        assert "file.txt" in files1
        assert "file.txt" in files2
        assert len(files1) == 1
        assert len(files2) == 1

    async def test_list_subdirectories(self, db):
        """Should list subdirectories within a directory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/parent/child1/file.txt", "content")
        await fs.write_file("/parent/child2/file.txt", "content")
        await fs.write_file("/parent/file.txt", "content")

        entries = await fs.readdir("/parent")
        assert "file.txt" in entries
        assert "child1" in entries
        assert "child2" in entries

    async def test_nested_directory_structures(self, db):
        """Should handle nested directory structures"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/a/b/c/d/file.txt", "deep content")
        files = await fs.readdir("/a/b/c/d")
        assert "file.txt" in files


@pytest.mark.asyncio
class TestFilesystemDeleteOperations:
    """Filesystem delete operations"""

    async def test_delete_existing_file(self, db):
        """Should delete an existing file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/delete-me.txt", "content")
        await fs.delete_file("/delete-me.txt")
        with pytest.raises(ErrnoException):
            await fs.read_file("/delete-me.txt")

    async def test_delete_nonexistent_file(self, db):
        """Should handle deleting non-existent file"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.delete_file("/non-existent.txt")

    async def test_delete_and_update_directory_listing(self, db):
        """Should delete file and update directory listing"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file1.txt", "content 1")
        await fs.write_file("/dir/file2.txt", "content 2")

        await fs.delete_file("/dir/file1.txt")

        files = await fs.readdir("/dir")
        assert "file1.txt" not in files
        assert "file2.txt" in files
        assert len(files) == 1

    async def test_recreate_deleted_file(self, db):
        """Should allow recreating deleted file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/recreate.txt", "original")
        await fs.delete_file("/recreate.txt")
        await fs.write_file("/recreate.txt", "new content")
        content = await fs.read_file("/recreate.txt")
        assert content == "new content"


@pytest.mark.asyncio
class TestFilesystemPathHandling:
    """Filesystem path handling"""

    async def test_paths_with_trailing_slashes(self, db):
        """Should handle paths with trailing slashes"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file.txt", "content")
        files1 = await fs.readdir("/dir")
        files2 = await fs.readdir("/dir/")
        assert files1 == files2

    async def test_paths_with_special_characters(self, db):
        """Should handle paths with special characters"""
        fs = await Filesystem.from_database(db)

        special_path = "/dir-with-dash/file_with_underscore.txt"
        await fs.write_file(special_path, "content")
        content = await fs.read_file(special_path)
        assert content == "content"


@pytest.mark.asyncio
class TestFilesystemIntegrity:
    """Filesystem integrity tests"""

    async def test_maintain_file_hierarchy_integrity(self, db):
        """Should maintain file hierarchy integrity"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/root.txt", "root")
        await fs.write_file("/dir1/file.txt", "dir1")
        await fs.write_file("/dir2/file.txt", "dir2")
        await fs.write_file("/dir1/subdir/file.txt", "subdir")

        assert await fs.read_file("/root.txt") == "root"
        assert await fs.read_file("/dir1/file.txt") == "dir1"
        assert await fs.read_file("/dir2/file.txt") == "dir2"
        assert await fs.read_file("/dir1/subdir/file.txt") == "subdir"

        root_files = await fs.readdir("/")
        assert "root.txt" in root_files
        assert "dir1" in root_files
        assert "dir2" in root_files

    async def test_multiple_files_same_name_different_directories(self, db):
        """Should support multiple files with same name in different directories"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir1/config.json", '{"version": 1}')
        await fs.write_file("/dir2/config.json", '{"version": 2}')

        assert await fs.read_file("/dir1/config.json") == '{"version": 1}'
        assert await fs.read_file("/dir2/config.json") == '{"version": 2}'


@pytest.mark.asyncio
class TestFilesystemStandaloneUsage:
    """Filesystem standalone usage tests"""

    async def test_basic_standalone_usage(self, db):
        """Should work as a standalone filesystem"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/test.txt", "standalone content")
        content = await fs.read_file("/test.txt")
        assert content == "standalone content"

    @pytest.mark.skip(
        reason="exercised SQLite-style independent in-memory databases; the "
        "PostgreSQL backend shares a single connection per test, so two "
        "independent stores cannot be created within one fixture"
    )
    async def test_maintain_isolation_between_instances(self, db):
        """Should maintain isolation between instances"""


@pytest.mark.asyncio
class TestFilesystemPersistence:
    """Filesystem persistence tests"""

    async def test_persist_across_instances(self, db):
        """Should persist data across Filesystem instances"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/persist.txt", "persistent content")

        new_fs = await Filesystem.from_database(db)
        content = await new_fs.read_file("/persist.txt")
        assert content == "persistent content"


@pytest.mark.asyncio
class TestFilesystemChunkSize:
    """Filesystem chunk size tests"""

    async def test_default_chunk_size(self, db):
        """Should have default chunk size of 4096"""
        fs = await Filesystem.from_database(db)

        assert fs.get_chunk_size() == 4096

    async def test_write_file_smaller_than_chunk_size(self, db):
        """Should write file smaller than chunk size"""
        fs = await Filesystem.from_database(db)

        # Write a file smaller than chunk_size (100 bytes)
        data = "x" * 100
        await fs.write_file("/small.txt", data)

        # Read it back
        read_data = await fs.read_file("/small.txt")
        assert len(read_data) == 100
        assert read_data == data

    async def test_write_file_exact_chunk_size(self, db):
        """Should write file exactly chunk size"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Write exactly chunk_size bytes
        data = bytes(i % 256 for i in range(chunk_size))
        await fs.write_file("/exact.txt", data)

        # Read it back
        read_data = await fs.read_file("/exact.txt", encoding=None)
        assert len(read_data) == chunk_size

    async def test_write_file_over_chunk_size(self, db):
        """Should write file one byte over chunk size"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Write chunk_size + 1 bytes
        data = bytes(i % 256 for i in range(chunk_size + 1))
        await fs.write_file("/overflow.txt", data)

        # Read it back
        read_data = await fs.read_file("/overflow.txt", encoding=None)
        assert len(read_data) == chunk_size + 1

    async def test_write_file_spanning_multiple_chunks(self, db):
        """Should write file spanning multiple chunks"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Write ~2.5 chunks worth of data
        data_size = int(chunk_size * 2.5)
        data = bytes(i % 256 for i in range(data_size))
        await fs.write_file("/multi.txt", data)

        # Read it back
        read_data = await fs.read_file("/multi.txt", encoding=None)
        assert len(read_data) == data_size


@pytest.mark.asyncio
class TestFilesystemDataIntegrity:
    """Filesystem data integrity tests"""

    async def test_roundtrip_data_byte_for_byte(self, db):
        """Should roundtrip data byte-for-byte"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Create data that spans chunk boundaries with identifiable patterns
        data_size = chunk_size * 3 + 123  # Odd size spanning 4 chunks

        data = bytes(i % 256 for i in range(data_size))
        await fs.write_file("/roundtrip.bin", data)

        read_data = await fs.read_file("/roundtrip.bin", encoding=None)
        assert len(read_data) == data_size
        assert read_data == data

    async def test_handle_binary_data_with_null_bytes(self, db):
        """Should handle binary data with null bytes"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Create data with null bytes at chunk boundaries
        data = bytearray(chunk_size * 2 + 100)
        # Put nulls at the chunk boundary
        data[chunk_size - 1] = 0
        data[chunk_size] = 0
        data[chunk_size + 1] = 0
        # Put some non-null bytes around
        data[chunk_size - 2] = 0xFF
        data[chunk_size + 2] = 0xFF

        await fs.write_file("/nulls.bin", bytes(data))
        read_data = await fs.read_file("/nulls.bin", encoding=None)

        assert read_data[chunk_size - 2] == 0xFF
        assert read_data[chunk_size - 1] == 0
        assert read_data[chunk_size] == 0
        assert read_data[chunk_size + 1] == 0
        assert read_data[chunk_size + 2] == 0xFF

    async def test_preserve_chunk_ordering(self, db):
        """Should preserve chunk ordering"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()
        # Create sequential bytes spanning multiple chunks
        data_size = chunk_size * 5
        data = bytes(i % 256 for i in range(data_size))
        await fs.write_file("/sequential.bin", data)

        read_data = await fs.read_file("/sequential.bin", encoding=None)

        # Verify every byte is in the correct position
        for i in range(data_size):
            assert read_data[i] == i % 256


@pytest.mark.asyncio
class TestFilesystemEdgeCases:
    """Filesystem edge case tests"""

    async def test_empty_file_with_zero_chunks(self, db):
        """Should handle empty file with zero chunks"""
        fs = await Filesystem.from_database(db)

        # Write empty file
        await fs.write_file("/empty.txt", "")

        # Read it back
        read_data = await fs.read_file("/empty.txt")
        assert read_data == ""

        # Verify size is 0
        stats = await fs.stat("/empty.txt")
        assert stats.size == 0

    async def test_overwrite_large_file_with_smaller(self, db):
        """Should overwrite large file with smaller file and clean up chunks"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()

        # Write initial large file (3 chunks)
        initial_data = bytes(i % 256 for i in range(chunk_size * 3))
        await fs.write_file("/overwrite.txt", initial_data)

        # Overwrite with smaller file (1 chunk)
        new_data = "x" * 100
        await fs.write_file("/overwrite.txt", new_data)

        # Verify old chunks are gone and new data is correct
        read_data = await fs.read_file("/overwrite.txt")
        assert read_data == new_data

        # Verify size is updated
        stats = await fs.stat("/overwrite.txt")
        assert stats.size == 100

    async def test_overwrite_small_file_with_larger(self, db):
        """Should overwrite small file with larger file"""
        fs = await Filesystem.from_database(db)

        chunk_size = fs.get_chunk_size()

        # Write initial small file (1 chunk)
        initial_data = "x" * 100
        await fs.write_file("/grow.txt", initial_data)

        # Overwrite with larger file (3 chunks)
        new_data = bytes(i % 256 for i in range(chunk_size * 3))
        await fs.write_file("/grow.txt", new_data)

        # Verify data is correct
        read_data = await fs.read_file("/grow.txt", encoding=None)
        assert len(read_data) == chunk_size * 3

    async def test_very_large_file(self, db):
        """Should handle very large file (1MB)"""
        fs = await Filesystem.from_database(db)

        # Write 1MB file
        data_size = 1024 * 1024
        data = bytes(i % 256 for i in range(data_size))
        await fs.write_file("/large.bin", data)

        read_data = await fs.read_file("/large.bin", encoding=None)
        assert len(read_data) == data_size


@pytest.mark.asyncio
class TestFilesystemStats:
    """Filesystem stats tests"""

    async def test_stat_file(self, db):
        """Should get file statistics"""
        fs = await Filesystem.from_database(db)

        content = "Hello, World!"
        await fs.write_file("/test.txt", content)

        stats = await fs.stat("/test.txt")
        assert stats.is_file()
        assert not stats.is_directory()
        assert stats.size == len(content)
        assert stats.ino > 0

    async def test_stat_directory(self, db):
        """Should get directory statistics"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file.txt", "content")

        stats = await fs.stat("/dir")
        assert stats.is_directory()
        assert not stats.is_file()

    async def test_stat_nonexistent_path(self, db):
        """Should throw error for non-existent path"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.stat("/nonexistent")


@pytest.mark.asyncio
class TestFilesystemMkdir:
    """Tests for mkdir() operation"""

    async def test_create_directory(self, db):
        """Should create a directory with mkdir()"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/newdir")
        entries = await fs.readdir("/")
        assert "newdir" in entries

    async def test_mkdir_throws_eexist_for_existing_directory(self, db):
        """Should throw EEXIST when mkdir() is called on an existing directory"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/exists")
        with pytest.raises(ErrnoException, match="EEXIST"):
            await fs.mkdir("/exists")

    async def test_mkdir_throws_enoent_for_missing_parent(self, db):
        """Should throw ENOENT when parent directory does not exist"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.mkdir("/missing-parent/child")


@pytest.mark.asyncio
class TestFilesystemRm:
    """Tests for rm() operation"""

    async def test_remove_file(self, db):
        """Should remove a file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/rmfile.txt", "content")
        await fs.rm("/rmfile.txt")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.read_file("/rmfile.txt")

    async def test_rm_force_does_not_throw_for_missing_file(self, db):
        """Should not throw when force=True and path does not exist"""
        fs = await Filesystem.from_database(db)

        # Should not raise
        await fs.rm("/does-not-exist", force=True)

    async def test_rm_throws_enoent_without_force(self, db):
        """Should throw ENOENT when force=False and path does not exist"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.rm("/does-not-exist")

    async def test_rm_throws_eisdir_for_directory_without_recursive(self, db):
        """Should throw EISDIR when trying to rm a directory without recursive"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/rmdir")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.rm("/rmdir")

    async def test_rm_recursive_removes_directory_tree(self, db):
        """Should remove a directory recursively"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/tree/a/b/c.txt", "content")
        await fs.rm("/tree", recursive=True)
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.readdir("/tree")
        root = await fs.readdir("/")
        assert "tree" not in root


@pytest.mark.asyncio
class TestFilesystemRmdir:
    """Tests for rmdir() operation"""

    async def test_remove_empty_directory(self, db):
        """Should remove an empty directory"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/emptydir")
        await fs.rmdir("/emptydir")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.readdir("/emptydir")
        root = await fs.readdir("/")
        assert "emptydir" not in root

    async def test_rmdir_throws_enotempty_for_non_empty_directory(self, db):
        """Should throw ENOTEMPTY when directory is not empty"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/nonempty/file.txt", "content")
        with pytest.raises(ErrnoException, match="ENOTEMPTY"):
            await fs.rmdir("/nonempty")

    async def test_rmdir_throws_enotdir_for_file(self, db):
        """Should throw ENOTDIR when path is a file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/afile", "content")
        with pytest.raises(ErrnoException, match="ENOTDIR"):
            await fs.rmdir("/afile")

    async def test_rmdir_throws_eperm_for_root(self, db):
        """Should throw EPERM when attempting to remove root"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="EPERM"):
            await fs.rmdir("/")


@pytest.mark.asyncio
class TestFilesystemRename:
    """Tests for rename() operation"""

    async def test_rename_file(self, db):
        """Should rename a file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/a.txt", "hello")
        await fs.rename("/a.txt", "/b.txt")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.read_file("/a.txt")
        content = await fs.read_file("/b.txt", "utf-8")
        assert content == "hello"

    async def test_rename_directory_preserves_contents(self, db):
        """Should rename a directory and preserve its contents"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/olddir/sub/file.txt", "content")
        await fs.rename("/olddir", "/newdir")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.readdir("/olddir")
        content = await fs.read_file("/newdir/sub/file.txt", "utf-8")
        assert content == "content"

    async def test_rename_overwrites_destination_file(self, db):
        """Should overwrite destination file if it exists"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/src.txt", "src")
        await fs.write_file("/dst.txt", "dst")
        await fs.rename("/src.txt", "/dst.txt")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.read_file("/src.txt")
        content = await fs.read_file("/dst.txt", "utf-8")
        assert content == "src"

    async def test_rename_throws_eisdir_for_file_to_directory(self, db):
        """Should throw EISDIR when renaming a file onto a directory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file.txt", "content")
        await fs.write_file("/file.txt", "content")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.rename("/file.txt", "/dir")

    async def test_rename_throws_enotdir_for_directory_to_file(self, db):
        """Should throw ENOTDIR when renaming a directory onto a file"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/somedir")
        await fs.write_file("/somefile", "content")
        with pytest.raises(ErrnoException, match="ENOTDIR"):
            await fs.rename("/somedir", "/somefile")

    async def test_rename_replaces_empty_directory(self, db):
        """Should replace an existing empty directory"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/fromdir")
        await fs.mkdir("/todir")
        await fs.rename("/fromdir", "/todir")
        root = await fs.readdir("/")
        assert "todir" in root
        assert "fromdir" not in root
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.readdir("/fromdir")

    async def test_rename_throws_enotempty_for_non_empty_destination(self, db):
        """Should throw ENOTEMPTY when replacing a non-empty directory"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/fromdir")
        await fs.write_file("/todir/file.txt", "content")
        with pytest.raises(ErrnoException, match="ENOTEMPTY"):
            await fs.rename("/fromdir", "/todir")

    async def test_rename_throws_eperm_for_root(self, db):
        """Should throw EPERM when attempting to rename root"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="EPERM"):
            await fs.rename("/", "/x")

    async def test_rename_throws_einval_for_directory_into_subdirectory(self, db):
        """Should throw EINVAL when renaming a directory into its own subdirectory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/cycle/sub/file.txt", "content")
        with pytest.raises(ErrnoException, match="EINVAL"):
            await fs.rename("/cycle", "/cycle/sub/moved")


@pytest.mark.asyncio
class TestFilesystemCopyFile:
    """Tests for copy_file() operation"""

    async def test_copy_file(self, db):
        """Should copy a file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/src.txt", "hello")
        await fs.copy_file("/src.txt", "/dst.txt")
        src_content = await fs.read_file("/src.txt", "utf-8")
        dst_content = await fs.read_file("/dst.txt", "utf-8")
        assert src_content == "hello"
        assert dst_content == "hello"

    async def test_copy_file_overwrites_destination(self, db):
        """Should overwrite destination if it exists"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/src.txt", "src")
        await fs.write_file("/dst.txt", "dst")
        await fs.copy_file("/src.txt", "/dst.txt")
        dst_content = await fs.read_file("/dst.txt", "utf-8")
        assert dst_content == "src"

    async def test_copy_file_throws_enoent_for_missing_source(self, db):
        """Should throw ENOENT when source does not exist"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.copy_file("/nope.txt", "/out.txt")

    async def test_copy_file_throws_enoent_for_missing_destination_parent(self, db):
        """Should throw ENOENT when destination parent does not exist"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/src3.txt", "content")
        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.copy_file("/src3.txt", "/missing/child.txt")

    async def test_copy_file_throws_eisdir_for_directory_source(self, db):
        """Should throw EISDIR when source is a directory"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/asrcdir")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.copy_file("/asrcdir", "/out2.txt")

    async def test_copy_file_throws_eisdir_for_directory_destination(self, db):
        """Should throw EISDIR when destination is a directory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/src4.txt", "content")
        await fs.mkdir("/adstdir")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.copy_file("/src4.txt", "/adstdir")

    async def test_copy_file_throws_einval_for_same_source_and_destination(self, db):
        """Should throw EINVAL when source and destination are the same"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/same.txt", "content")
        with pytest.raises(ErrnoException, match="EINVAL"):
            await fs.copy_file("/same.txt", "/same.txt")


@pytest.mark.asyncio
class TestFilesystemAccess:
    """Tests for access() operation"""

    async def test_access_existing_file(self, db):
        """Should resolve when a file exists"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/exists.txt", "content")
        # Should not raise
        await fs.access("/exists.txt")

    async def test_access_existing_directory(self, db):
        """Should resolve when a directory exists"""
        fs = await Filesystem.from_database(db)

        await fs.mkdir("/existsdir")
        # Should not raise
        await fs.access("/existsdir")

    async def test_access_throws_enoent_for_nonexistent_path(self, db):
        """Should throw ENOENT when path does not exist"""
        fs = await Filesystem.from_database(db)

        with pytest.raises(ErrnoException, match="ENOENT"):
            await fs.access("/does-not-exist")


@pytest.mark.asyncio
class TestFilesystemErrorCodes:
    """Tests for error code validation on existing methods"""

    async def test_write_file_throws_eisdir_for_directory(self, db):
        """Should throw EISDIR when attempting to write to a directory path"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file.txt", "content")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.write_file("/dir", "nope")

    async def test_write_file_throws_enotdir_for_file_in_path(self, db):
        """Should throw ENOTDIR when a parent path component is a file"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/a", "file-content")
        with pytest.raises(ErrnoException, match="ENOTDIR"):
            await fs.write_file("/a/b.txt", "child")

    async def test_read_file_throws_eisdir_for_directory(self, db):
        """Should throw EISDIR when attempting to read a directory path"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/dir/file.txt", "content")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.read_file("/dir")

    async def test_readdir_throws_enotdir_for_file(self, db):
        """Should throw ENOTDIR when attempting to readdir a file path"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/notadir.txt", "content")
        with pytest.raises(ErrnoException, match="ENOTDIR"):
            await fs.readdir("/notadir.txt")

    async def test_unlink_throws_eisdir_for_directory(self, db):
        """Should throw EISDIR when attempting to unlink a directory"""
        fs = await Filesystem.from_database(db)

        await fs.write_file("/adir/file.txt", "content")
        with pytest.raises(ErrnoException, match="EISDIR"):
            await fs.unlink("/adir")
