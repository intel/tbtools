declare -r TYPE_ARRAY=array
declare -r TYPE_OBJ=obj

declare -rA type_codes=([$TYPE_ARRAY]=a [$TYPE_OBJ]=A)

serialize_obj()
{
	local -nr obj_ref=$1
	local obj_values=()
	local k

	for k in "${!obj_ref[@]}"; do
		obj_values+=("[${k@Q}]=${obj_ref[$k]@Q}")
	done

	echo "${obj_values[*]}"
}

serialize_array()
{
	local -nr array_ref=$1
	local array_name=${2:-$1}
	local array_values=()
	local i

	for ((i = 0; i < ${#array_ref[@]}; i++)); do
		array_values+=("${array_ref[$i]@Q}")
	done

	echo "${array_values[*]}"
}


__get_obj_values()
{
	local -nr __list_obj_ref=$1
	local obj_values=()
	local k

	for k in "${!__list_obj_ref[@]}"; do
		obj_values+=("[\"$k\"]=\"${__list_obj_ref["$k"]}\"")
	done

	echo "${obj_values[*]}"
}

get_obj_values()
{
	echo "($(__get_obj_values $1))"
}

concat_obj_values()
{
	echo "($(__get_obj_values $1) $(__get_obj_values $2))"
}

get_array_elements()
{
	local -n __get_array_elements_ref=$1
	local array_name=${2:-$1}
	local array_values=()
	local i

	for ((i = 0; i < ${#__get_array_elements_ref[@]}; i++)); do
		array_values+=("[$i]=\"${__get_array_elements_ref["$i"]}\"")
	done

	echo "(${array_values[*]})"
}

describe_obj()
{
	local -nr __describe_obj_ref=$1
	local obj_name=${2:-$1}

	echo "$obj_name=$(get_obj_values __describe_obj_ref)"
}

describe_array()
{
	local -n __describe_array_ref=$1
	local array_name=${2:-$1}

	echo "$array_name=$(get_array_elements __describe_array_ref)"
}

assert_var_defined()
{
	local var=$1

	[[ "$(declare -p $var 2> /dev/null)" ]] && return 0

	err_exit "$var is not defined"
}

assert_type()
{
	local -r obj_name=$1
	local -r type_name=$2
	local -r type_code=${type_codes["$type_name"]}

	assert_var_defined "$obj_name"

	[[ "$(declare -p $obj_name 2> /dev/null)" =~ "declare -${type_code}" ]] && return 0

	err_exit "Type of $obj_name is not a(n) "$type_name" type"
}

assert_obj_type()
{
	assert_type "$1" "$TYPE_OBJ"
}

assert_array_type()
{
	assert_type "$1" "$TYPE_ARRAY"
}

assert_array_size()
{
	local name=$1
	local -nr __arr_assert=$2
	local size=$3

	[ "${#__arr_assert[@]}" -eq $size ] && return 0

	err_exit "Array $name has incorrect size: ${#__arr_assert[@]} (expected $size)\n"
}

assert_empty_obj()
{
	local -rn __assert_empty_obj_ref=$1
	local -r obj_name=${2:-$1}

	assert_obj_type "$obj_name"

	[ -z "${!__assert_empty_obj_ref[*]}" ] && return 0

	err_exit "Object not empty: $(describe_obj __assert_empty_obj_ref "$obj_name")"
}

assert_empty_array()
{
	local -rn __assert_empty_array_ref=$1
	local -r array_name=${2:-$1}

	assert_array_type "$array_name"

	[ -z "${__assert_empty_array_ref+set}" ] && return 0

	if [ "${#__assert_empty_array_ref[@]}" -ne 0 ]; then
		err_exit "Array not empty: $(describe_array __assert_empty_array_ref "$array_name")"
	fi
}

check_keys()
{
	local -rn exp_keys=$1
	local -rn obj=$2
	local -r obj_name=$3
	local key

	if [ ${#exp_keys[@]} -ne ${#obj[@]} ]; then
		err_exit "Obj \"$obj_name\" has key mismatch: ${!obj[@]} (expected:${exp_keys[@]})"
	fi

	for key in "${exp_keys[@]}"; do
		if ! [ "${obj["$key"]+x}" ]; then
			err_exit "Missing key in obj \"$obj_name\": \"$key\""
		fi
	done
}

obj_init()
{
	local -rn __obj_ref=$1

	assert_empty_obj "$2"

	eval "__obj_ref=$3"
}

array_init()
{
	local -rn __array_ref=$1

	assert_empty_array "$1"

	eval "__array_ref=$2"
}
