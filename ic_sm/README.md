#ic_sm
## Working Name: Ironclaw Secret Manager
###### via the agent REPL or web UI, ironclaw has (sometimes) options to interface with secrets.

### For example:
1. You can ask the agent to list the secrets, and it will do so.
2. You can interface with the encrypted key of the master secret store - can use with interfacing with the database.
3. You can ask the agent to remove a secret.

##### The predicament


###### IronClaw is a bare bones project still and it needs a lot of tools built out for it.

###### In the meantime, I vibe coded a python script that lets me insert secrets directly into the databse, being encrypted right from the beginning.

####### protip: you dont even need to restart the agent after you load in a new secret with this wonderful script.


######## its in python. not too long.

Requirements are in requirements.txt in scripts dir

requirements are for both versions of script. libsql and postgresql supported

### Example use
```
$ python3 insert_secret_libsql.py secret_name secret_data_goes_here
```


baud was here
starforce was here!
