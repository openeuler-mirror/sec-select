# Testing SecAFS

## pjdfstest

```bash
git clone git@github.com:pjd/pjdfstest.git
cd pjdfstest
autoreconf -ifs
./configure
make pjdfstest
sudo make install
sudo dnf install perl-Test-Harness
mkdir -p ../secafs-testing
cd ../secafs-testing
secafs init testing
mkdir mnt
sudo su
secafs mount testing ./mnt
cd mnt
prove -rv ../../pjdfstest/tests/ 2>&1 | tee /tmp/pjdfstest.log
```

## xftests

First, build the `secafs` executable and install it locally including the `mount.fuse.secafs` helper:

```bash
cd cli
cargo build --release
cp target/release/secafs /usr/local/bin
cp scripts/mount.fuse.secafs /sbin
```

Then, clone the xfstests repo:

```bash
git clone git://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git
```

Configure the filesystem under test:

```bash
cat local.config
export FSTYP=fuse
export FUSE_SUBTYP=.secafs
export TEST_DEV=<database file>
export TEST_DIR=<mount directory>
```

Then, run xfstests:

```bash
sudo ./check -g quick generic/
```
