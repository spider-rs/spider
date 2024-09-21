[![Gitter](https://img.shields.io/gitter/room/MARC-FS/MARC-FS.svg)](https://gitter.im/MARC-FS/Lobby?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)
[![build status](https://gitlab.com/Kanedias/MARC-FS/badges/master/build.svg)](https://gitlab.com/Kanedias/MARC-FS/commits/master)
[![License](https://img.shields.io/aur/license/marcfs-git.svg)](https://www.gnu.org/licenses/gpl-3.0.html)

MARC-FS
===========
Mail.ru Cloud filesystem written for FUSE

Synopsis
--------
This is an implementation of a simple filesystem with all calls and hooks needed for normal file operations. After mounting it you'll be provided access to all your cloud files remotely stored on Mail.ru Cloud as if they were local ones. You should keep in mind that this is a network-driven FS and so it will never be as fast as any local one, but having a folder connected as remote drive in 9P/GNU Hurd fashion can be convenient at a times.

**Bear in mind that this project is still in its infancy, sudden errors/crashes/memory leaks may occur.**

Features
--------

- cloud storage is represented as local folder
- `rm`, `cp`, `ls`, `rmdir`, `touch`, `grep` and so on are working
- filesystem stats are working, can check with `df`
- multithreaded, you can work with multiple files at once
- support for files > 2GB by seamless splitting/joining uploaded/downloaded files

Installation & Usage
--------------------
You should have cmake and g++ with C++14 support at hand.
MARC-FS also requires `libfuse` (obviously), `libcurl` (min 7.34) and `pthread` libraries. Once you have all this, do as usual:

    $ git clone --recursive https://gitlab.com/Kanedias/MARC-FS.git
    $ cd MARC-FS
    $ mkdir build
    $ cd build && cmake ..
    $ make
    $ # here goes the step where you actually go and register on mail.ru website to obtain cloud storage and auth info
    $ ./marcfs /path/to/mount/folder -o username=your.email@mail.ru,password=your.password,cachedir=/path/to/cache

If you want your files on Mail.ru Cloud to be encrypted, you may use nested EncFS filesystem to achieve this:

    $ ./marcfs /path/to/mount/folder -o username=your.email@mail.ru,password=your.password
    $ mkdir /path/to/mount/folder/encrypted # needed only once when you init your EncFS
    $ encfs --no-default-flags /path/to/mount/folder/encrypted /path/to/decrypted/dir
    $ cp whatever /path/to/decrypted/dir
    $ # at this point encrypted data will appear in Cloud Mail.ru storage

If you want to use rsync to synchronize local and remote sides, use `--sizes-only` option. 
Rsync compares mtime and size of file by default, but Mail.ru Cloud saves only seconds in mtime, 
which causes false-positives and reuploads of identical files:

    $ rsync -av --delete --size-only /path/to/local/folder/ ~/path/to/mount/folder

To unmount previously mounted share, make sure no one uses it and execute:

    $ # if you mounted encfs previously, first unmount it
    $ # fusermount -u /path/to/mount/folder/encrypted
    $ fusermount -u /path/to/mount/folder

If you want to get a shared link to the file, you should create a file with special name, `*.marcfs-link`

    $ # suppose we want to get a public link to file 'picture.png'
    $ touch picture.png.marcfs-link
    $ cat picture.png.marcfs-link
    /path/to/file/pictire.png: https://cloud.mail.ru/public/LINK/ADDRESS

Files with size > 2G will show up as series of shared links for each part. 
After getting the link special file can be safely removed.

Notes
-----

#### External configuration ####

If you don't want to type credentials on the command line you can use config file for that.
The file is `~/.config/marcfs/config.json` (default [XDG basedir spec](https://standards.freedesktop.org/basedir-spec/basedir-spec-latest.html)).
You can override its' location via `-o conffile=/path/to/config` option. Example config:

```json
{
    "username": "user@mail.ru",
    "password": "password",
    "cachedir": "/absolute/path"
    "proxyurl": "http://localhost:3128"
}
```

#### Cache dir ####

MARC-FS has two modes of operation. If no cachedir option is given, it stores all intermediate download/upload 
data directly in memory. If you copy large files as HD movies or ISO files, it may eat up your RAM pretty quickly,
so be careful. This one is useful if you want to copy your photo library to/from the cloud - this will actually take
a lot less time than with second option.

If cachedir option is given, MARC-FS stores all intermediate data there. It means, all files that are currently open
in some process, copied/read or being edited - will have their data stored in this dir. This may sound like plenty 
of space, but most software execute file operations sequentally, so in case of copying large media library on/from 
the cloud you won't need more free space than largest one of the files occupies.

API references
--------------
- There is no official Mail.ru Cloud API reference, everything is reverse-engineered. You may refer to [Doxygen API comments](https://gitlab.com/Kanedias/MARC-FS/blob/master/marc_api.h) to grasp concept of what's going on.
- FUSE: [API overview](https://www.cs.hmc.edu/~geoff/classes/hmc.cs135.201109/homework/fuse/fuse_doc.html) - used to implement FS calls
- cURL: [API overview](https://curl.haxx.se/docs/) - used to interact with Mail.ru Cloud REST API

Motivation
----------
Mail.ru is one of largest Russian social networks. It provides mail services, hosting, gaming platforms and, incidentally, cloud services, similar to Dropbox, NextCloud etc.

Once upon a time Mail.ru did a discount for this cloud solution and provided beta testers (and your humble servant among them) with free 1 TiB storage.

And so... A holy place is never empty.

Bugs & Known issues
-------------------
1. Temporary
  - SOme issues may arise if you delete/move file that is currently copied or read. Please report such bugs here.
  - big memory footprint due to 
      - SSL engine sessions - tend to become bigger with time (WIP)
      - heap fragmentation (WIP)
      - MADV_FREE - lazy memory reclaiming in Linux > 4.5 (not a bug actually)
  - On RHEL-based distros (CentOS/Fedora) you may need `NSS_STRICT_NOFORK=DISABLED` environment variable (see [this](https://gitlab.com/Kanedias/MARC-FS/issues/6) and [this](https://bugzilla.redhat.com/show_bug.cgi?id=1317691))
2. Principal (Mail.ru Cloud API limitations)
  - No extended attr/chmod support, all files on storage are owned by you
  - No atime/ctime support, only mtime is stored
  - No mtime support for directories, expect all of them to have `Jan 1 1970` date in `ls`
  - No `Transfer-Encoding: chunked` support for POST **requests** in cloud nginx (`chunkin on`/`proxy_request_buffering` options in `nginx`/`tengine` config), so files are read fully into memory before uploading

Contributions
------------
You may create merge request or bug/enhancement issue right here on GitLab, or send formatted patch via e-mail. For details see CONTRIBUTING.md file in this repo. 
Audits from code style and security standpoint are also much appreciated.

License
-------

    Copyright (C) 2016-2017  Oleg `Kanedias` Chernovskiy

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.
