# - Try to find the CHECK libraries
#  Once done this will define
#
#  CHECK_FOUND - system has check
#  CHECK_INCLUDE_DIRS - the check include directory
#  CHECK_LIBRARIES - check library
#
#  Copyright (c) 2007 Daniel Gollub <gollub@b1-systems.de>
#  Copyright (c) 2007-2009 Bjoern Ricks  <bjoern.ricks@gmail.com>
#
#  Redistribution and use is allowed according to the terms of the New
#  BSD license.
#  For details see the accompanying COPYING-CMAKE-SCRIPTS file.


INCLUDE( FindPkgConfig )

IF ( Check_FIND_REQUIRED )
	SET( _pkgconfig_REQUIRED "REQUIRED" )
ELSE( Check_FIND_REQUIRED )
	SET( _pkgconfig_REQUIRED "" )
ENDIF ( Check_FIND_REQUIRED )

IF ( CHECK_MIN_VERSION )
	PKG_SEARCH_MODULE( CHECK ${_pkgconfig_REQUIRED} check>=${CHECK_MIN_VERSION} )
ELSE ( CHECK_MIN_VERSION )
	PKG_SEARCH_MODULE( CHECK ${_pkgconfig_REQUIRED} check )
ENDIF ( CHECK_MIN_VERSION )

# Look for CHECK include dir and libraries
IF( NOT CHECK_FOUND AND NOT PKG_CONFIG_FOUND )

	FIND_PATH( CHECK_INCLUDE_DIRS check.h )

	FIND_LIBRARY( CHECK_LIBRARIES NAMES check )

	IF ( CHECK_INCLUDE_DIRS AND CHECK_LIBRARIES )
		SET( CHECK_FOUND 1 )
		IF ( NOT Check_FIND_QUIETLY )
			MESSAGE ( STATUS "Found CHECK: ${CHECK_LIBRARIES}" )
		ENDIF ( NOT Check_FIND_QUIETLY )
	ELSE ( CHECK_INCLUDE_DIRS AND CHECK_LIBRARIES )
		IF ( Check_FIND_REQUIRED )
			MESSAGE( FATAL_ERROR "Could NOT find CHECK" )
		ELSE ( Check_FIND_REQUIRED )
			IF ( NOT Check_FIND_QUIETLY )
				MESSAGE( STATUS "Could NOT find CHECK" )
			ENDIF ( NOT Check_FIND_QUIETLY )
		ENDIF ( Check_FIND_REQUIRED )
	ENDIF ( CHECK_INCLUDE_DIRS AND CHECK_LIBRARIES )
ENDIF( NOT CHECK_FOUND AND NOT PKG_CONFIG_FOUND )

# Hide advanced variables from CMake GUIs
MARK_AS_ADVANCED( CHECK_INCLUDE_DIRS CHECK_LIBRARIES )

