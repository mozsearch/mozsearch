# Using Docker with Searchfox

## Debugging Provisioning Failure

Did the `docker build` step fall over?

You can run the following to get a list of images:
```
docker image ls
```

This should list the known images sorted by creation time, with the most recent
image being at the top.  You'll want to copy the `IMAGE ID`.  You can then run
a shell in the container via the following, replacing `$IMAGE_ID` with the
relevant image id.

```
docker run -it --entrypoint bash $IMAGE_ID
```
